//! Bounded, fixed-endpoint WASAPI capture, independent of ordinary dictation.
use super::queue::CaptureSender;
use super::types::{AudioEncoding, AudioFormat, CaptureEvent, CapturePacket, Source};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::TrySendError;

const MAX_PACKET_BYTES: usize = 4 * 1024 * 1024;
const SILENT: u32 = 2;

fn parse_wave_format(bytes: &[u8]) -> Result<AudioFormat, String> {
    const INVALID: &str = "meeting_capture_format_invalid";
    const UNSUPPORTED: &str = "meeting_capture_format_unsupported";
    if bytes.len() < 18 {
        return Err(INVALID.into());
    }
    let word = |offset| u16::from_le_bytes([bytes[offset], bytes[offset + 1]]);
    let dword = |offset| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
    let tag = word(0);
    let extension = usize::from(word(16));
    if extension > 4096 || bytes.len() < 18 + extension {
        return Err(INVALID.into());
    }
    let bits = word(14);
    let (encoding, valid_bits, channel_mask) = match tag {
        1 => (AudioEncoding::Pcm, bits, 0),
        3 => (AudioEncoding::Float, bits, 0),
        0xfffe => {
            if extension < 22 {
                return Err(INVALID.into());
            }
            let guid_tail = [
                0, 0, 0, 0, 0, 0x10, 0, 0x80, 0, 0, 0xaa, 0, 0x38, 0x9b, 0x71,
            ];
            if bytes[25..40] != guid_tail {
                return Err(UNSUPPORTED.into());
            }
            let encoding = match bytes[24] {
                1 => AudioEncoding::Pcm,
                3 => AudioEncoding::Float,
                _ => return Err(UNSUPPORTED.into()),
            };
            (encoding, word(18), dword(20))
        }
        _ => return Err(UNSUPPORTED.into()),
    };
    let format = AudioFormat {
        sample_rate: dword(4),
        channels: word(2),
        bits_per_sample: bits,
        valid_bits,
        channel_mask,
        encoding,
    };
    let align = validate_format(&format)?;
    if usize::from(word(12)) != align
        || u64::from(dword(8)) != u64::from(format.sample_rate) * align as u64
    {
        return Err(INVALID.into());
    }
    Ok(format)
}

fn validate_format(format: &AudioFormat) -> Result<usize, String> {
    if format.channels == 0
        || format.channels > 32
        || format.sample_rate < 8_000
        || format.sample_rate > 384_000
        || format.valid_bits == 0
        || format.valid_bits > format.bits_per_sample
    {
        return Err("meeting_capture_format_invalid".into());
    }
    match format.encoding {
        AudioEncoding::Pcm if matches!(format.bits_per_sample, 8 | 16 | 24 | 32) => {}
        AudioEncoding::Float if format.bits_per_sample == 32 && format.valid_bits == 32 => {}
        _ => return Err("meeting_capture_format_unsupported".into()),
    }
    Ok(usize::from(format.channels) * usize::from(format.bits_per_sample / 8))
}

/// Copies a borrowed WASAPI packet while it is owned by this worker.
///
/// # Safety
/// For non-silent, nonempty packets `data` must point to `frames * block_align`
/// readable bytes, as guaranteed by a successful IAudioCaptureClient::GetBuffer.
#[allow(clippy::too_many_arguments)] // Mirrors the native packet and timing contract.
unsafe fn copy_packet(
    source: Source,
    epoch: u32,
    sequence: u64,
    device_frame_position: u64,
    qpc_100ns: u64,
    frames: u32,
    flags: u32,
    format: &AudioFormat,
    data: *const u8,
) -> Result<Option<CapturePacket>, String> {
    // AUDCLNT_S_BUFFER_EMPTY is projected as Ok(()); none of its out parameters
    // other than frames may be consumed and no ReleaseBuffer is required.
    if frames == 0 {
        return Ok(None);
    }
    let align = validate_format(format)?;
    let length = (frames as usize)
        .checked_mul(align)
        .filter(|length| *length <= MAX_PACKET_BYTES)
        .ok_or("meeting_capture_packet_too_large")?;
    let bytes = if flags & SILENT != 0 {
        Vec::new()
    } else {
        if data.is_null() {
            return Err("meeting_capture_null_packet".into());
        }
        let bytes = unsafe { std::slice::from_raw_parts(data, length) }.to_vec();
        if format.encoding == AudioEncoding::Float
            && bytes
                .chunks_exact(4)
                .any(|chunk| !f32::from_le_bytes(chunk.try_into().unwrap()).is_finite())
        {
            return Err("meeting_capture_nonfinite_sample".into());
        }
        bytes
    };
    Ok(Some(CapturePacket {
        source,
        epoch,
        sequence,
        device_frame_position,
        qpc_100ns,
        frames,
        flags,
        format: format.clone(),
        data: bytes,
    }))
}

fn deliver_event(
    sender: &CaptureSender,
    event: CaptureEvent,
    stop: &AtomicBool,
) -> Result<(), String> {
    match sender.try_send(event) {
        Ok(()) => Ok(()),
        Err(error) => {
            stop.store(true, Ordering::Release);
            Err(match error {
                TrySendError::Full(_) => "meeting_capture_queue_full",
                TrySendError::Disconnected(_) => "meeting_capture_consumer_gone",
            }
            .into())
        }
    }
}

fn qpc_to_100ns(ticks: i64, frequency: i64) -> Result<u64, String> {
    if ticks < 0 || frequency <= 0 {
        return Err("meeting_capture_clock_invalid".into());
    }
    u64::try_from(i128::from(ticks) * 10_000_000 / i128::from(frequency))
        .map_err(|_| "meeting_capture_clock_invalid".into())
}

#[cfg(target_os = "windows")]
pub use native::{list_devices, now_qpc_100ns, start_capture, CaptureHandle};

#[cfg(not(target_os = "windows"))]
pub struct CaptureHandle;
#[cfg(not(target_os = "windows"))]
impl CaptureHandle {
    pub fn stop(self) -> Result<(), String> {
        Err("meeting_capture_platform_unsupported".into())
    }
}
#[cfg(not(target_os = "windows"))]
pub fn list_devices() -> Result<Vec<super::types::DeviceInfo>, String> {
    Err("meeting_capture_platform_unsupported".into())
}
#[cfg(not(target_os = "windows"))]
pub fn now_qpc_100ns() -> Result<u64, String> {
    Err("meeting_capture_platform_unsupported".into())
}
#[cfg(not(target_os = "windows"))]
pub fn start_capture(
    _config: super::types::CaptureConfig,
    _sender: CaptureSender,
    _stop: std::sync::Arc<AtomicBool>,
) -> Result<CaptureHandle, String> {
    Err("meeting_capture_platform_unsupported".into())
}

#[cfg(target_os = "windows")]
mod native {
    use super::super::types::{CaptureConfig, DeviceInfo};
    use super::*;
    use std::sync::mpsc::{channel, sync_channel, Receiver, Sender};
    use std::sync::Arc;
    use std::thread::{self, JoinHandle};
    use std::time::{Duration, Instant};
    use windows::core::{Interface, PCWSTR};
    use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
    use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
    use windows::Win32::Media::Audio::{
        eCapture, eCommunications, eRender, EDataFlow, IAudioCaptureClient, IAudioClient,
        IMMDevice, IMMDeviceEnumerator, IMMEndpoint, MMDeviceEnumerator, AUDCLNT_SHAREMODE_SHARED,
        AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_LOOPBACK, DEVICE_STATE_ACTIVE,
        WAVEFORMATEX,
    };
    use windows::Win32::System::Com::StructuredStorage::{
        PropVariantClear, PropVariantToStringAlloc, PROPVARIANT,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
        COINIT_MULTITHREADED, STGM_READ,
    };
    use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
    use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

    const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
    const HEARTBEAT_PERIOD: Duration = Duration::from_secs(1);
    const MICROPHONE_STALL: Duration = Duration::from_secs(5);
    const MAX_DRAIN_PACKETS: usize = 128;

    // Only operation and HRESULT enter error codes; endpoint IDs/names never do.
    fn os_error(operation: &'static str, error: windows::core::Error) -> String {
        format!("meeting_capture_{operation}_{:08x}", error.code().0 as u32)
    }

    struct ComApartment;
    impl ComApartment {
        fn new() -> Result<Self, String> {
            unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
                .ok()
                .map_err(|e| os_error("com", e))?;
            Ok(Self)
        }
    }
    impl Drop for ComApartment {
        fn drop(&mut self) {
            unsafe { CoUninitialize() };
        }
    }

    struct TaskMemory<T>(*mut T);
    impl<T> Drop for TaskMemory<T> {
        fn drop(&mut self) {
            unsafe { CoTaskMemFree(Some(self.0.cast())) };
        }
    }
    struct Variant(PROPVARIANT);
    impl Drop for Variant {
        fn drop(&mut self) {
            let _ = unsafe { PropVariantClear(&mut self.0) };
        }
    }
    struct AudioEvent(HANDLE);
    impl AudioEvent {
        fn new() -> Result<Self, String> {
            unsafe { CreateEventW(None, false, false, None) }
                .map(Self)
                .map_err(|e| os_error("event_create", e))
        }
    }
    impl Drop for AudioEvent {
        fn drop(&mut self) {
            let _ = unsafe { CloseHandle(self.0) };
        }
    }

    fn device_id(device: &IMMDevice) -> Result<String, String> {
        let value = unsafe { device.GetId() }.map_err(|e| os_error("device_id", e))?;
        let owned = TaskMemory(value.0);
        if owned.0.is_null() {
            return Err("meeting_capture_device_id_null".into());
        }
        unsafe { PCWSTR(owned.0).to_string() }
            .map_err(|_| "meeting_capture_device_id_invalid".into())
    }

    fn device_name(device: &IMMDevice) -> Result<String, String> {
        let store = unsafe { device.OpenPropertyStore(STGM_READ) }
            .map_err(|e| os_error("device_properties", e))?;
        let value = Variant(
            unsafe { store.GetValue(&PKEY_Device_FriendlyName) }
                .map_err(|e| os_error("device_name", e))?,
        );
        let value = unsafe { PropVariantToStringAlloc(&value.0) }
            .map_err(|e| os_error("device_name_string", e))?;
        let owned = TaskMemory(value.0);
        if owned.0.is_null() {
            return Err("meeting_capture_device_name_null".into());
        }
        unsafe { PCWSTR(owned.0).to_string() }
            .map_err(|_| "meeting_capture_device_name_invalid".into())
    }

    fn enumerator() -> Result<IMMDeviceEnumerator, String> {
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
            .map_err(|e| os_error("enumerator", e))
    }

    /// Lists active endpoints on a dedicated COM thread without opening a stream.
    pub fn list_devices() -> Result<Vec<DeviceInfo>, String> {
        thread::Builder::new()
            .name("meeting-device-list".into())
            .spawn(|| {
                let _com = ComApartment::new()?;
                let enumerator = enumerator()?;
                let mut devices = Vec::new();
                for (flow, source) in [(eCapture, Source::Microphone), (eRender, Source::Remote)] {
                    let default_id =
                        unsafe { enumerator.GetDefaultAudioEndpoint(flow, eCommunications) }
                            .ok()
                            .and_then(|device| device_id(&device).ok());
                    let collection =
                        unsafe { enumerator.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE) }
                            .map_err(|e| os_error("enumerate", e))?;
                    let count = unsafe { collection.GetCount() }
                        .map_err(|e| os_error("device_count", e))?;
                    for index in 0..count {
                        let device = unsafe { collection.Item(index) }
                            .map_err(|e| os_error("device_item", e))?;
                        let id = device_id(&device)?;
                        let name = device_name(&device)?;
                        devices.push(DeviceInfo {
                            is_default: default_id.as_ref() == Some(&id),
                            id,
                            name,
                            source,
                        });
                    }
                }
                Ok(devices)
            })
            .map_err(|_| "meeting_capture_thread_spawn".to_string())?
            .join()
            .map_err(|_| "meeting_capture_thread_panicked".to_string())?
    }

    /// Same 100 ns QPC domain that WASAPI already uses for packet timestamps.
    pub fn now_qpc_100ns() -> Result<u64, String> {
        let mut ticks = 0;
        let mut frequency = 0;
        unsafe { QueryPerformanceCounter(&mut ticks) }.map_err(|e| os_error("qpc", e))?;
        unsafe { QueryPerformanceFrequency(&mut frequency) }
            .map_err(|e| os_error("qpc_frequency", e))?;
        qpc_to_100ns(ticks, frequency)
    }

    struct NativeStream {
        // Drop order: service/client before event, all before the worker apartment.
        capture: IAudioCaptureClient,
        client: IAudioClient,
        event: AudioEvent,
        device: IMMDevice,
        format: AudioFormat,
        max_frames: u32,
        started: bool,
    }

    impl NativeStream {
        fn open(id: &str, source: Source) -> Result<Self, String> {
            if id.is_empty() || id.len() > 32_768 || id.contains('\0') {
                return Err("meeting_capture_device_id_invalid".into());
            }
            let wide: Vec<u16> = id.encode_utf16().chain(Some(0)).collect();
            let enumerator = enumerator()?;
            let device = unsafe { enumerator.GetDevice(PCWSTR(wide.as_ptr())) }
                .map_err(|e| os_error("open_device", e))?;
            if unsafe { device.GetState() }.map_err(|e| os_error("device_state", e))?
                != DEVICE_STATE_ACTIVE
            {
                return Err("meeting_capture_device_inactive".into());
            }
            let endpoint: IMMEndpoint = device.cast().map_err(|e| os_error("endpoint", e))?;
            let flow = unsafe { endpoint.GetDataFlow() }.map_err(|e| os_error("device_flow", e))?;
            let expected: EDataFlow = if source == Source::Microphone {
                eCapture
            } else {
                eRender
            };
            if flow != expected {
                return Err("meeting_capture_device_wrong_direction".into());
            }
            let client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None) }
                .map_err(|e| os_error("activate", e))?;
            let format_ptr = TaskMemory(
                unsafe { client.GetMixFormat() }.map_err(|e| os_error("mix_format", e))?,
            );
            if format_ptr.0.is_null() {
                return Err("meeting_capture_format_null".into());
            }
            let header: WAVEFORMATEX = unsafe { format_ptr.0.read_unaligned() };
            let extension = usize::from(header.cbSize);
            if extension > 4096 {
                return Err("meeting_capture_format_invalid".into());
            }
            let format = parse_wave_format(unsafe {
                std::slice::from_raw_parts(format_ptr.0.cast::<u8>(), 18 + extension)
            })?;
            let flags = AUDCLNT_STREAMFLAGS_EVENTCALLBACK
                | if source == Source::Remote {
                    AUDCLNT_STREAMFLAGS_LOOPBACK
                } else {
                    0
                };
            unsafe { client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 0, 0, format_ptr.0, None) }
                .map_err(|e| os_error("initialize", e))?;
            let max_frames =
                unsafe { client.GetBufferSize() }.map_err(|e| os_error("buffer_size", e))?;
            if max_frames == 0
                || (max_frames as usize)
                    .checked_mul(validate_format(&format)?)
                    .filter(|bytes| *bytes <= MAX_PACKET_BYTES)
                    .is_none()
            {
                return Err("meeting_capture_buffer_too_large".into());
            }
            let event = AudioEvent::new()?;
            unsafe { client.SetEventHandle(event.0) }.map_err(|e| os_error("set_event", e))?;
            let capture =
                unsafe { client.GetService() }.map_err(|e| os_error("capture_service", e))?;
            Ok(Self {
                capture,
                client,
                event,
                device,
                format,
                max_frames,
                started: false,
            })
        }

        fn start(&mut self) -> Result<(), String> {
            unsafe { self.client.Start() }.map_err(|e| os_error("start", e))?;
            self.started = true;
            Ok(())
        }

        fn stop(&mut self) -> Result<(), String> {
            if self.started {
                unsafe { self.client.Stop() }.map_err(|e| os_error("stop", e))?;
                self.started = false;
            }
            Ok(())
        }

        fn health(&self) -> Result<(), String> {
            if unsafe { self.device.GetState() }.map_err(|e| os_error("device_state", e))?
                != DEVICE_STATE_ACTIVE
            {
                return Err("meeting_capture_device_inactive".into());
            }
            // Loopback legitimately produces no packets while nothing is playing.
            // Querying the existing client detects invalidation without opening or
            // switching endpoints, and never synthesizes an audio reference.
            unsafe { self.client.GetCurrentPadding() }.map_err(|e| os_error("stream_health", e))?;
            Ok(())
        }

        fn drain(
            &self,
            source: Source,
            epoch: u32,
            sequence: &mut u64,
            sender: &CaptureSender,
            stop: &AtomicBool,
            final_drain: bool,
        ) -> Result<bool, String> {
            let mut any = false;
            for _ in 0..MAX_DRAIN_PACKETS {
                if !final_drain && stop.load(Ordering::Acquire) {
                    return Ok(any);
                }
                let next = unsafe { self.capture.GetNextPacketSize() }
                    .map_err(|e| os_error("next_packet", e))?;
                if next == 0 {
                    return Ok(any);
                }
                if next > self.max_frames {
                    return Err("meeting_capture_frame_count_invalid".into());
                }
                let mut data = std::ptr::null_mut();
                let mut frames = 0;
                let mut flags = 0;
                let mut device_position = 0;
                let mut qpc = 0;
                unsafe {
                    self.capture.GetBuffer(
                        &mut data,
                        &mut frames,
                        &mut flags,
                        Some(&mut device_position),
                        Some(&mut qpc),
                    )
                }
                .map_err(|e| os_error("get_buffer", e))?;
                // BUFFER_EMPTY is a success HRESULT. Never touch its other output
                // values or pair it with a ReleaseBuffer of a nonexistent packet.
                if frames == 0 {
                    return Ok(any);
                }
                let lease = PacketLease {
                    capture: &self.capture,
                    frames: Some(frames),
                };
                let packet = if frames > self.max_frames {
                    Err("meeting_capture_frame_count_invalid".into())
                } else {
                    unsafe {
                        copy_packet(
                            source,
                            epoch,
                            *sequence,
                            device_position,
                            qpc,
                            frames,
                            flags,
                            &self.format,
                            data,
                        )
                    }
                };
                // No queue operation occurs while the borrowed engine buffer is held.
                lease.release()?;
                if let Some(packet) = packet? {
                    deliver_event(sender, CaptureEvent::Packet(packet), stop)?;
                    *sequence = sequence
                        .checked_add(1)
                        .ok_or("meeting_capture_sequence_overflow")?;
                    any = true;
                }
            }
            // A broken driver must not monopolize stop/join indefinitely.
            Err("meeting_capture_drain_limit".into())
        }
    }
    impl Drop for NativeStream {
        fn drop(&mut self) {
            let _ = self.stop();
        }
    }

    struct PacketLease<'a> {
        capture: &'a IAudioCaptureClient,
        frames: Option<u32>,
    }
    impl PacketLease<'_> {
        fn release(mut self) -> Result<(), String> {
            let frames = self.frames.take().unwrap();
            unsafe { self.capture.ReleaseBuffer(frames) }.map_err(|e| os_error("release_buffer", e))
        }
    }
    impl Drop for PacketLease<'_> {
        fn drop(&mut self) {
            if let Some(frames) = self.frames.take() {
                let _ = unsafe { self.capture.ReleaseBuffer(frames) };
            }
        }
    }

    enum Notice {
        Ready,
        Started,
        Failed(String),
    }

    fn worker(
        id: String,
        source: Source,
        epoch: u32,
        sender: &CaptureSender,
        stop: &AtomicBool,
        notices: &Sender<Notice>,
        start: Receiver<bool>,
    ) -> Result<(), String> {
        let _com = ComApartment::new()?;
        let mut stream = NativeStream::open(&id, source)?;
        notices
            .send(Notice::Ready)
            .map_err(|_| "meeting_capture_startup_cancelled")?;
        match start.recv_timeout(STARTUP_TIMEOUT) {
            Ok(true) if !stop.load(Ordering::Acquire) => {}
            _ => return Ok(()),
        }
        stream.start()?;
        notices
            .send(Notice::Started)
            .map_err(|_| "meeting_capture_startup_cancelled")?;
        let mut sequence = 0;
        let mut last_packet = Instant::now();
        let mut last_heartbeat = Instant::now();
        let result = (|| {
            while !stop.load(Ordering::Acquire) {
                let status = unsafe { WaitForSingleObject(stream.event.0, 100) };
                if status != WAIT_OBJECT_0 && status != WAIT_TIMEOUT {
                    return Err("meeting_capture_event_wait_failed".into());
                }
                if stream.drain(source, epoch, &mut sequence, sender, stop, false)? {
                    last_packet = Instant::now();
                }
                if source == Source::Microphone && last_packet.elapsed() >= MICROPHONE_STALL {
                    return Err("meeting_capture_microphone_stalled".into());
                }
                if last_heartbeat.elapsed() >= HEARTBEAT_PERIOD {
                    stream.health()?;
                    deliver_event(
                        sender,
                        CaptureEvent::Heartbeat {
                            source,
                            qpc_100ns: now_qpc_100ns()?,
                        },
                        stop,
                    )?;
                    last_heartbeat = Instant::now();
                }
            }
            Ok(())
        })();
        let stop_result = stream.stop();
        // On failure preserve the first error; on ordinary stop keep the tail.
        // A consumer still drains the channel until stop() joins both workers.
        let tail_result = if result.is_ok() && stop_result.is_ok() {
            stream
                .drain(source, epoch, &mut sequence, sender, stop, true)
                .map(|_| ())
        } else {
            Ok(())
        };
        result.and(stop_result).and(tail_result)
    }

    /// Owns both worker threads. The shared stop flag becomes true on any error;
    /// the coordinator must observe it and call stop() to collect a terminal error
    /// even when a full packet queue could not accept the best-effort Error event.
    pub struct CaptureHandle {
        stop: Arc<AtomicBool>,
        workers: Vec<JoinHandle<Result<(), String>>>,
    }
    impl CaptureHandle {
        fn join(&mut self) -> Result<(), String> {
            self.stop.store(true, Ordering::Release);
            let mut first_error = None;
            for worker in self.workers.drain(..) {
                let result = worker
                    .join()
                    .unwrap_or_else(|_| Err("meeting_capture_thread_panicked".into()));
                if first_error.is_none() {
                    first_error = result.err();
                }
            }
            first_error.map_or(Ok(()), Err)
        }
        pub fn stop(mut self) -> Result<(), String> {
            self.join()
        }
    }
    impl Drop for CaptureHandle {
        fn drop(&mut self) {
            let _ = self.join();
        }
    }

    /// Both endpoints initialize before either stream starts. A partial failure
    /// stops and joins both workers; no device fallback or playback mute is used.
    pub fn start_capture(
        config: CaptureConfig,
        sender: CaptureSender,
        stop: Arc<AtomicBool>,
    ) -> Result<CaptureHandle, String> {
        if stop.load(Ordering::Acquire) {
            return Err("meeting_capture_cancelled".into());
        }
        let mut handle = CaptureHandle {
            stop: stop.clone(),
            workers: Vec::with_capacity(2),
        };
        let (notices, receiver) = channel();
        let mut gates = Vec::with_capacity(2);
        for (id, source) in [
            (config.microphone_id, Source::Microphone),
            (config.render_id, Source::Remote),
        ] {
            let (gate, start) = sync_channel(1);
            let sender = sender.clone();
            let stop = stop.clone();
            let notices = notices.clone();
            let epoch = config.epoch;
            let thread = thread::Builder::new()
                .name(
                    match source {
                        Source::Microphone => "meeting-microphone",
                        Source::Remote => "meeting-loopback",
                    }
                    .into(),
                )
                .spawn(move || {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        worker(id, source, epoch, &sender, &stop, &notices, start)
                    }))
                    .unwrap_or_else(|_| Err("meeting_capture_thread_panicked".into()));
                    if let Err(code) = &result {
                        stop.store(true, Ordering::Release);
                        let _ = notices.send(Notice::Failed(code.clone()));
                        let _ = sender.try_send(CaptureEvent::Error {
                            source,
                            code: code.clone(),
                        });
                    }
                    result
                });
            match thread {
                Ok(thread) => {
                    handle.workers.push(thread);
                    gates.push(gate);
                }
                Err(_) => {
                    drop(gates);
                    let _ = handle.join();
                    return Err("meeting_capture_thread_spawn".into());
                }
            }
        }
        drop(notices);
        let startup = (|| {
            for _ in 0..2 {
                match receiver.recv_timeout(STARTUP_TIMEOUT) {
                    Ok(Notice::Ready) => {}
                    Ok(Notice::Failed(code)) => return Err(code),
                    _ => return Err("meeting_capture_initialize_timeout".into()),
                }
            }
            for gate in &gates {
                gate.send(true)
                    .map_err(|_| "meeting_capture_startup_cancelled")?;
            }
            for _ in 0..2 {
                match receiver.recv_timeout(STARTUP_TIMEOUT) {
                    Ok(Notice::Started) => {}
                    Ok(Notice::Failed(code)) => return Err(code),
                    _ => return Err("meeting_capture_start_timeout".into()),
                }
            }
            if stop.load(Ordering::Acquire) {
                return Err("meeting_capture_startup_cancelled".into());
            }
            Ok(())
        })();
        if let Err(error) = startup {
            drop(gates);
            let join_result = handle.join();
            return Err(join_result.err().unwrap_or(error));
        }
        Ok(handle)
    }
}

#[cfg(test)]
#[path = "capture_tests.rs"]
mod tests;
