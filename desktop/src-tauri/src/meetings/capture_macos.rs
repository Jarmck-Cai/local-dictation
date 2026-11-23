//! ScreenCaptureKit microphone/system audio capture in a single host-clock domain.
use super::super::types::{CaptureConfig, CapturePacket, DeviceInfo};
use super::{
    complete_interval, copy_packet, deliver_event, verify_health_continuity, AudioEncoding,
    AudioFormat, CaptureEvent, CaptureSender, Source,
};
use std::{
    ffi::{c_char, c_int, c_void, CStr, CString},
    ptr::NonNull,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

const SYSTEM_AUDIO_ID: &str = "macos-system-audio";
const MAX_FRAMES: u32 = 8192;
const SOURCES: [Source; 2] = [Source::Microphone, Source::Remote];

type DeviceCallback = unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char, bool);
type AudioCallback = unsafe extern "C" fn(*mut c_void, u32, u32, u32, u64, *const f32);
type ErrorCallback = unsafe extern "C" fn(*mut c_void, u32);

extern "C" {
    fn qingshu_macos_meeting_supported() -> bool;
    fn qingshu_macos_meeting_clock() -> u64;
    fn qingshu_macos_meeting_devices(context: *mut c_void, callback: DeviceCallback);
    fn qingshu_macos_meeting_start(
        microphone: *const c_char,
        context: *mut c_void,
        audio: AudioCallback,
        failed: ErrorCallback,
        status: *mut c_int,
    ) -> *mut c_void;
    fn qingshu_macos_meeting_stop(handle: *mut c_void) -> c_int;
}

fn native_error(status: u32) -> &'static str {
    match status {
        1 => "meeting_capture_macos_version",
        2 => "meeting_capture_macos_permission",
        3 => "meeting_capture_device_inactive",
        5 => "meeting_capture_format_unsupported",
        6 => "meeting_capture_clock_invalid",
        7 => "meeting_capture_macos_interrupted",
        8 => "meeting_capture_macos_stop",
        9 => "meeting_capture_macos_timeout",
        _ => "meeting_capture_macos_start",
    }
}

pub(in crate::meetings) fn supported() -> bool {
    // The native helper performs an availability check before using any 15.0 APIs.
    unsafe { qingshu_macos_meeting_supported() }
}

pub(super) fn now_qpc_100ns() -> Result<u64, String> {
    match unsafe { qingshu_macos_meeting_clock() } {
        0 => Err("meeting_capture_clock_invalid".into()),
        ticks => Ok(ticks),
    }
}

pub fn list_devices() -> Result<Vec<DeviceInfo>, String> {
    if !supported() {
        return Err("meeting_capture_macos_version".into());
    }
    unsafe extern "C" fn append(
        context: *mut c_void,
        id: *const c_char,
        name: *const c_char,
        is_default: bool,
    ) {
        if id.is_null() || name.is_null() {
            return;
        }
        // The native enumerator calls synchronously, with UTF-8 strings scoped
        // to this callback and the exclusive Vec borrow passed below.
        let devices = unsafe { &mut *context.cast::<Vec<DeviceInfo>>() };
        let id = unsafe { CStr::from_ptr(id) }.to_string_lossy().into_owned();
        let name = unsafe { CStr::from_ptr(name) }
            .to_string_lossy()
            .into_owned();
        devices.push(DeviceInfo {
            id,
            name,
            source: Source::Microphone,
            is_default,
        });
    }
    let mut devices = Vec::new();
    unsafe { qingshu_macos_meeting_devices((&mut devices as *mut Vec<DeviceInfo>).cast(), append) };
    devices.push(DeviceInfo {
        id: SYSTEM_AUDIO_ID.into(),
        name: "System audio".into(),
        source: Source::Remote,
        is_default: true,
    });
    Ok(devices)
}

#[derive(Default)]
struct Track {
    next_sequence: u64,
    next_position: u64,
    last_start: u64,
    last_end: u64,
    rate: u32,
}

impl Track {
    fn position(&self, time: u64, rate: u32) -> Result<u64, String> {
        if self.next_sequence == 0 {
            return Ok(0);
        }
        if rate != self.rate {
            return Err("meeting_capture_format_changed".into());
        }
        if time <= self.last_start || time.saturating_add(20_000) < self.last_end {
            return Err("meeting_capture_clock_invalid".into());
        }
        // Add frame positions for true playback gaps. Minor timestamp jitter
        // retains contiguous indices; alignment checks the native timestamps.
        let gap = time.saturating_sub(self.last_end);
        let missing_frames = if gap > 20_000 {
            gap * u64::from(rate) / 10_000_000
        } else {
            0
        };
        Ok(self.next_position + missing_frames)
    }

    fn observe(&mut self, packet: &CapturePacket) {
        self.next_sequence += 1;
        self.next_position = packet.device_frame_position + u64::from(packet.frames);
        self.last_start = packet.qpc_100ns;
        self.rate = packet.format.sample_rate;
        self.last_end =
            self.last_start + u64::from(packet.frames) * 10_000_000 / u64::from(self.rate);
    }
}

#[derive(Default)]
struct Boundary {
    start: u64,
    end: Option<u64>,
    error: Option<String>,
    tracks: [Track; 2],
}

struct Session {
    boundary: Mutex<Boundary>,
    sender: CaptureSender,
    stop: Arc<AtomicBool>,
    epoch: u32,
}

impl Session {
    fn fail(&self, boundary: &mut Boundary, source: Source, code: String) {
        if boundary.error.is_none() {
            let _ = self.sender.try_send(CaptureEvent::Error {
                source,
                code: code.clone(),
            });
            boundary.error = Some(code);
        }
        self.stop.store(true, Ordering::Release);
    }

    fn heartbeat(&self, at: u64) -> Result<(), String> {
        for source in SOURCES {
            deliver_event(
                &self.sender,
                CaptureEvent::Heartbeat {
                    source,
                    qpc_100ns: at,
                },
                &self.stop,
            )?;
        }
        Ok(())
    }
}

unsafe extern "C" fn on_error(context: *mut c_void, status: u32) {
    // Native stop synchronously disconnects this pointer before releasing Session.
    let session = unsafe { &*context.cast::<Session>() };
    if let Ok(mut state) = session.boundary.lock() {
        session.fail(&mut state, Source::Remote, native_error(status).into());
    }
}

unsafe extern "C" fn on_audio(
    context: *mut c_void,
    source: u32,
    rate: u32,
    frames: u32,
    time: u64,
    samples: *const f32,
) {
    let session = unsafe { &*context.cast::<Session>() };
    let Some(&source_kind) = SOURCES.get(source as usize) else {
        return;
    };
    let Ok(mut state) = session.boundary.lock() else {
        session.stop.store(true, Ordering::Release);
        return;
    };
    if state.start == 0
        || state.error.is_some()
        || time < state.start
        || state.end.is_some_and(|end| time >= end)
        || session.stop.load(Ordering::Acquire)
    {
        return;
    }
    let result = (|| {
        if frames == 0 || frames > MAX_FRAMES || samples.is_null() {
            return Err("meeting_capture_frame_count_invalid".into());
        }
        let track = &mut state.tracks[source as usize];
        let pos = track.position(time, rate)?;
        let format = AudioFormat {
            sample_rate: rate,
            channels: 1,
            bits_per_sample: 32,
            valid_bits: 32,
            channel_mask: 0,
            encoding: AudioEncoding::Float,
        };
        // The bridge supplies frames tightly packed f32 samples for this call.
        let packet = unsafe {
            copy_packet(
                source_kind,
                session.epoch,
                track.next_sequence,
                pos,
                time,
                frames,
                0,
                &format,
                samples.cast(),
            )
        }?
        .ok_or("meeting_capture_frame_count_invalid")?;
        track.observe(&packet);
        deliver_event(&session.sender, CaptureEvent::Packet(packet), &session.stop)
    })();
    if let Err(code) = result {
        session.fail(&mut state, source_kind, code);
    }
}

struct NativeStream(Option<NonNull<c_void>>);
// The retained Objective-C object serializes its callbacks independently. Rust
// moves sole ownership of the stop handle to the meeting actor's stop worker.
unsafe impl Send for NativeStream {}

impl NativeStream {
    fn stop(&mut self) -> Result<(), String> {
        if let Some(handle) = self.0.take() {
            let status = unsafe { qingshu_macos_meeting_stop(handle.as_ptr()) };
            if status != 0 {
                return Err(native_error(status as u32).into());
            }
        }
        Ok(())
    }
}
impl Drop for NativeStream {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

pub struct CaptureHandle {
    native: NativeStream,
    session: Arc<Session>,
    heartbeat: Option<JoinHandle<()>>,
    start: u64,
}

impl CaptureHandle {
    pub fn start_qpc_100ns(&self) -> u64 {
        self.start
    }

    fn join_heartbeat(&mut self) -> Result<(), String> {
        if let Some(beat) = self.heartbeat.take() {
            beat.thread().unpark();
            beat.join().map_err(|_| "meeting_capture_thread_panicked")?;
        }
        Ok(())
    }

    pub fn stop(mut self) -> Result<u64, String> {
        let end = now_qpc_100ns()?;
        self.session
            .boundary
            .lock()
            .map_err(|_| "meeting_capture_boundary_lock")?
            .end = Some(end);
        let stopped = self.native.stop();
        self.join_heartbeat()?;
        let state = self
            .session
            .boundary
            .lock()
            .map_err(|_| "meeting_capture_boundary_lock")?;
        if let Some(error) = &state.error {
            return Err(error.clone());
        }
        if self.session.stop.load(Ordering::Acquire) {
            return Err("meeting_capture_cancelled".into());
        }
        // Missing microphone packets must never become a healthy silent track.
        let last_mic = state.tracks[0].last_end;
        if last_mic == 0 || end.saturating_sub(last_mic) > 15_000_000 {
            return Err("meeting_capture_macos_microphone".into());
        }
        complete_interval(
            self.start,
            Some(end),
            stopped,
            &self.session.sender,
            &self.session.stop,
        )
    }
}

impl Drop for CaptureHandle {
    fn drop(&mut self) {
        self.session.stop.store(true, Ordering::Release);
        let _ = self.native.stop();
        let _ = self.join_heartbeat();
    }
}

fn monitor(session: Arc<Session>, mut previous: u64) {
    loop {
        thread::park_timeout(Duration::from_millis(750));
        let Ok(mut state) = session.boundary.lock() else {
            session.stop.store(true, Ordering::Release);
            return;
        };
        if state.end.is_some() || session.stop.load(Ordering::Acquire) {
            return;
        }
        let beat = now_qpc_100ns().and_then(|now| {
            verify_health_continuity(previous, now)?;
            let last_audio = state.tracks[0].last_end.max(state.start);
            if now.saturating_sub(last_audio) > 15_000_000 {
                return Err("meeting_capture_macos_microphone".into());
            }
            session.heartbeat(now)?;
            previous = now;
            Ok(())
        });
        if let Err(code) = beat {
            session.fail(&mut state, Source::Microphone, code);
            return;
        }
    }
}

pub fn start_capture(
    config: CaptureConfig,
    sender: CaptureSender,
    stop: Arc<AtomicBool>,
) -> Result<CaptureHandle, String> {
    if config.render_id != SYSTEM_AUDIO_ID {
        return Err("meeting_capture_device_inactive".into());
    }
    let microphone =
        CString::new(config.microphone_id).map_err(|_| "meeting_capture_device_inactive")?;
    let session = Arc::new(Session {
        boundary: Mutex::new(Boundary::default()),
        sender,
        stop,
        epoch: config.epoch,
    });
    let mut status = 0;
    let pointer = unsafe {
        qingshu_macos_meeting_start(
            microphone.as_ptr(),
            Arc::as_ptr(&session).cast_mut().cast(),
            on_audio,
            on_error,
            &mut status,
        )
    };
    let native = NativeStream(Some(
        NonNull::new(pointer).ok_or_else(|| native_error(status as u32))?,
    ));
    let start = {
        let mut state = session
            .boundary
            .lock()
            .map_err(|_| "meeting_capture_boundary_lock")?;
        if let Some(code) = &state.error {
            return Err(code.clone());
        }
        let now = now_qpc_100ns()?;
        session.heartbeat(now)?;
        state.start = now;
        now
    };
    let beat_session = Arc::clone(&session);
    let heartbeat = thread::Builder::new()
        .name("macos-meeting-health".into())
        .spawn(move || monitor(beat_session, start))
        .map_err(|_| "meeting_thread_spawn")?;
    Ok(CaptureHandle {
        native,
        session,
        heartbeat: Some(heartbeat),
        start,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_silence_gaps_without_inventing_a_timebase() {
        let mut track = Track::default();
        assert_eq!(track.position(9_000_000, 48_000), Ok(0));
        track.next_sequence = 1;
        track.next_position = 480;
        track.last_start = 9_000_000;
        track.last_end = 9_100_000;
        track.rate = 48_000;
        assert_eq!(track.position(9_110_000, 48_000), Ok(480));
        assert_eq!(track.position(9_200_000, 48_000), Ok(960));
        assert!(track.position(9_001_000, 48_000).is_err());
        assert!(track.position(9_100_000, 44_100).is_err());
    }
}
