// Included by check-meeting-runtime-soak.py; controlled audio only, no devices.
use meetings::{
    contracts::MeetingSummary, queue::CaptureSender, runtime::*, store::MeetingStore, types::*,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
const BASE: u64 = 10_000_000;
const FRAMES: u32 = 480;
const BYTES: usize = 3840;
#[derive(Default, serde::Serialize)]
struct Stats {
    sent: [u64; 2],
    consumed: [u64; 2],
    consumed_frames: [u64; 2],
    consumed_bytes: [u64; 2],
    producer_late_us: u64,
    consume_lag_us: u64,
    append_us: u64,
    checkpoint_us: u64,
    queue_peak_payload_bytes: usize,
    reads: u64,
    read_us: u64,
    read_errors: u64,
    failure: Option<String>,
}
fn index(s: Source) -> usize {
    if s == Source::Microphone {
        0
    } else {
        1
    }
}
fn payload(source: Source, sequence: u64) -> Vec<u8> {
    (0..960)
        .flat_map(|n| {
            let v = (((sequence % 4093) as usize + n) % 251 + 1) as f32 / 256.0;
            (if source == Source::Microphone { v } else { -v }).to_le_bytes()
        })
        .collect()
}
fn packet(source: Source, sequence: u64) -> CapturePacket {
    CapturePacket {
        source,
        epoch: 0,
        sequence,
        device_frame_position: sequence * FRAMES as u64,
        qpc_100ns: BASE + sequence * 100_000,
        frames: FRAMES,
        flags: 0,
        format: AudioFormat {
            sample_rate: 48000,
            channels: 2,
            bits_per_sample: 32,
            valid_bits: 32,
            channel_mask: 3,
            encoding: AudioEncoding::Float,
        },
        data: payload(source, sequence),
    }
}
fn devices() -> Vec<DeviceInfo> {
    [Source::Microphone, Source::Remote]
        .into_iter()
        .map(|source| DeviceInfo {
            id: format!("synthetic-{}", index(source)),
            name: format!("Controlled source {}", index(source)),
            source,
            is_default: true,
        })
        .collect()
}
struct Factory {
    seconds: u64,
    stats: Arc<Mutex<Stats>>,
    armed: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
    start: Arc<Mutex<Option<Instant>>>,
}
struct Session {
    worker: thread::JoinHandle<Result<u64, String>>,
    _sender: CaptureSender,
}
impl CaptureFactory for Factory {
    fn start(
        &self,
        _: CaptureConfig,
        sender: CaptureSender,
        stop: Arc<AtomicBool>,
    ) -> Result<Box<dyn CaptureSession>, String> {
        let (seconds, stats, armed, done, start) = (
            self.seconds,
            self.stats.clone(),
            self.armed.clone(),
            self.done.clone(),
            self.start.clone(),
        );
        let tx = sender.clone();
        let worker = thread::spawn(move || {
            while !armed.load(Ordering::Acquire) {
                thread::sleep(Duration::from_millis(1));
            }
            let began = Instant::now();
            *start.lock().unwrap() = Some(began);
            let result: Result<u64, String> = (|| {
                for sequence in 0..seconds * 100 {
                    let target = began + Duration::from_millis(sequence * 10);
                    if let Some(wait) = target.checked_duration_since(Instant::now()) {
                        thread::sleep(wait);
                    }
                    if stop.load(Ordering::Acquire) {
                        return Err("soak_unexpected_capture_stop".into());
                    }
                    let late = Instant::now().saturating_duration_since(target).as_micros() as u64;
                    for source in [Source::Microphone, Source::Remote] {
                        tx.try_send(CaptureEvent::Packet(packet(source, sequence)))
                            .map_err(|_| "soak_send_failed")?;
                        let mut s = stats.lock().unwrap();
                        s.sent[index(source)] += 1;
                        s.producer_late_us = s.producer_late_us.max(late);
                        s.queue_peak_payload_bytes =
                            s.queue_peak_payload_bytes.max(tx.peak_payload_bytes());
                    }
                    if sequence % 100 == 0 {
                        for source in [Source::Microphone, Source::Remote] {
                            tx.try_send(CaptureEvent::Heartbeat {
                                source,
                                qpc_100ns: BASE + sequence * 100_000,
                            })
                            .map_err(|_| "soak_heartbeat_failed")?;
                        }
                    }
                }
                // The final packet starts 10ms before the requested wall time.
                if let Some(wait) =
                    (began + Duration::from_secs(seconds)).checked_duration_since(Instant::now())
                {
                    thread::sleep(wait);
                }
                Ok(BASE + seconds * 10_000_000)
            })();
            if let Err(error) = &result {
                stats.lock().unwrap().failure = Some(error.clone());
            }
            done.store(true, Ordering::Release);
            result
        });
        Ok(Box::new(Session {
            worker,
            _sender: sender,
        }))
    }
}
impl CaptureSession for Session {
    fn start_qpc_100ns(&self) -> u64 {
        BASE
    }
    fn stop(self: Box<Self>) -> Result<u64, String> {
        self.worker.join().map_err(|_| "soak_producer_panicked")?
    }
}
fn services(capture: Arc<dyn CaptureFactory>) -> RuntimeServices {
    RuntimeServices {
        devices: Arc::new(|| Ok(devices())),
        capture,
        acquire_recording: Arc::new(|| Ok(Box::new(()))),
        process: Arc::new(|_, _, _, _| Err("soak_asr_not_invoked".into())),
        emit: Arc::new(|_: MeetingSummary| {}),
    }
}
fn memory() -> (usize, usize) {
    #[repr(C)]
    struct Counters {
        cb: u32,
        faults: u32,
        peak: usize,
        working: usize,
        paged_peak: usize,
        paged: usize,
        nonpaged_peak: usize,
        nonpaged: usize,
        pagefile: usize,
        pagefile_peak: usize,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut std::ffi::c_void;
    }
    #[link(name = "psapi")]
    unsafe extern "system" {
        fn GetProcessMemoryInfo(p: *mut std::ffi::c_void, c: *mut Counters, size: u32) -> i32;
    }
    let mut c: Counters = unsafe { std::mem::zeroed() };
    c.cb = std::mem::size_of::<Counters>() as u32;
    assert_ne!(
        unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) },
        0
    );
    (c.working, c.peak)
}
fn verify(store: &MeetingStore, id: &str, count: u64) -> (u64, u64, u64) {
    let m = store.load(id).unwrap();
    let mut seq = [0u64; 2];
    let mut bytes = 0;
    for meta in &m.chunks {
        assert!(meta.finalized);
        let mut reader = store.open_reader_for_chunk(id, meta).unwrap();
        while let Some(p) = reader.next_packet().unwrap() {
            let n = index(p.source);
            assert_eq!(p.source, meta.source);
            assert_eq!(p.sequence, seq[n]);
            assert_eq!(p.device_frame_position, p.sequence * 480);
            assert_eq!(p.frames, 480);
            assert_eq!(p.qpc_100ns, BASE + p.sequence * 100_000);
            assert_eq!(p.format.sample_rate, 48000);
            assert_eq!(p.format.channels, 2);
            assert_eq!(p.format.encoding, AudioEncoding::Float);
            assert_eq!(p.format.bits_per_sample, 32);
            assert_eq!(p.data, payload(p.source, p.sequence));
            seq[n] += 1;
            bytes += p.data.len() as u64;
        }
    }
    assert_eq!(seq, [count; 2]);
    (m.chunks.len() as u64, seq.iter().sum::<u64>() * 480, bytes)
}
fn copy_meeting(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        if e.file_type().unwrap().is_file() {
            fs::copy(e.path(), to.join(e.file_name())).unwrap();
        }
    }
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let arg = |key: &str| {
        args.windows(2)
            .find(|w| w[0] == key)
            .map(|w| w[1].clone())
            .unwrap()
    };
    let root = PathBuf::from(arg("--root"));
    let seconds: u64 = arg("--seconds").parse().unwrap();
    let readers: usize = arg("--readers").parse().unwrap();
    assert!((1..=86400).contains(&seconds));
    assert!((1..=8).contains(&readers));
    // Refuse accidental reuse: never delete an existing fixture directory.
    assert!(!root.exists(), "use a fresh synthetic root");
    fs::create_dir_all(root.join("meetings")).unwrap();
    let root = fs::canonicalize(root).unwrap();
    let stats = Arc::new(Mutex::new(Stats::default()));
    let armed = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));
    let start = Arc::new(Mutex::new(None));
    let factory = Arc::new(Factory {
        seconds,
        stats: stats.clone(),
        armed: armed.clone(),
        done: done.clone(),
        start: start.clone(),
    });
    let store = MeetingStore::new(root.join("meetings")).unwrap();
    let runtime = Arc::new(
        MeetingRuntime::new(
            store.clone(),
            root.join("meetings"),
            services(factory.clone()),
        )
        .unwrap(),
    );
    let (obs_stats, obs_start) = (stats.clone(), start.clone());
    runtime.observe_capture(Arc::new(
        move |source, qpc, frames, bytes, append_us, writer| {
            let elapsed = obs_start.lock().unwrap().unwrap().elapsed().as_micros() as u64;
            let mut s = obs_stats.lock().unwrap();
            let n = index(source);
            s.consumed[n] += 1;
            s.consumed_frames[n] += frames as u64;
            s.consumed_bytes[n] += bytes as u64;
            s.consume_lag_us = s
                .consume_lag_us
                .max(elapsed.saturating_sub((qpc - BASE) / 10));
            s.append_us = s.append_us.max(append_us);
            s.checkpoint_us = s.checkpoint_us.max(writer.max_checkpoint_us);
        },
    ));
    let row = runtime
        .start(
            &"controlled synthetic metadata ".repeat(100),
            "synthetic-0",
            "synthetic-1",
        )
        .unwrap();
    let read_stop = Arc::new(AtomicBool::new(false));
    let mut workers = vec![];
    for _ in 0..readers {
        let (r, s, stop) = (runtime.clone(), stats.clone(), read_stop.clone());
        workers.push(thread::spawn(move || {
            let mut tick = 0;
            while !stop.load(Ordering::Acquire) {
                for _ in 0..if tick % 10 == 0 { 25 } else { 1 } {
                    let began = Instant::now();
                    let list = r.list();
                    let current = r.current();
                    let mut s = s.lock().unwrap();
                    s.reads += 1;
                    s.read_us = s.read_us.max(began.elapsed().as_micros() as u64);
                    if list.is_err() || current.is_err() {
                        s.read_errors += 1;
                    }
                }
                tick += 1;
                for _ in 0..10 {
                    if stop.load(Ordering::Acquire) {
                        break;
                    }
                    thread::sleep(Duration::from_millis(100));
                }
            }
        }));
    }
    let wall_start = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    armed.store(true, Ordering::Release);
    let mut last = Instant::now() - Duration::from_secs(10);
    while !done.load(Ordering::Acquire) {
        if last.elapsed() >= Duration::from_secs(10) {
            println!(
                "{}",
                serde_json::json!({"phase":"recording","unix":SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs_f64(),"memory":memory(),"stats":&*stats.lock().unwrap()})
            );
            last = Instant::now();
        }
        if runtime
            .current()
            .unwrap()
            .is_some_and(|r| r.status == MeetingStatus::Interrupted)
        {
            panic!("unexpected recording interruption");
        }
        thread::sleep(Duration::from_millis(50));
    }
    let elapsed = start.lock().unwrap().unwrap().elapsed().as_secs_f64();
    let stopped = runtime.stop(&row.id).unwrap();
    read_stop.store(true, Ordering::Release);
    for w in workers {
        w.join().unwrap();
    }
    assert!(elapsed >= seconds as f64);
    assert_eq!(stopped.status, MeetingStatus::Recorded);
    let manifest = store.load(&row.id).unwrap();
    assert_eq!(manifest.capture_complete, Some(true));
    assert!(manifest.capture_error.is_none());
    {
        let s = stats.lock().unwrap();
        assert_eq!(s.sent, [seconds * 100; 2]);
        assert_eq!(s.consumed, s.sent);
        assert_eq!(s.consumed_frames, [seconds * 48000; 2]);
        assert_eq!(s.consumed_bytes, [seconds * 100 * BYTES as u64; 2]);
        assert_eq!(s.read_errors, 0);
        assert!(s.failure.is_none());
        assert!(s.queue_peak_payload_bytes <= MEETING_CAPTURE_QUEUE_MAX_BYTES);
    }
    let actual: std::collections::HashSet<_> = fs::read_dir(root.join("meetings").join(&row.id))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("chunk-"))
        .collect();
    let expected: std::collections::HashSet<_> =
        manifest.chunks.iter().map(|c| c.file.clone()).collect();
    assert_eq!(actual, expected);
    println!(
        "{}",
        serde_json::json!({"phase":"verify_packets","elapsed_seconds":elapsed})
    );
    let (chunks, frames, bytes) = verify(&store, &row.id, seconds * 100);
    let clone_root = root.join("recovery-copy");
    let clone_dir = clone_root.join(&row.id);
    copy_meeting(&root.join("meetings").join(&row.id), &clone_dir);
    let clone_store = MeetingStore::new(&clone_root).unwrap();
    let mut m = clone_store.load(&row.id).unwrap();
    m.status = MeetingStatus::Recording;
    m.capture_complete = None;
    m.chunks[0].finalized = false;
    fs::write(
        clone_dir.join("manifest.json"),
        serde_json::to_vec(&m).unwrap(),
    )
    .unwrap();
    let recovered = clone_store.recover(&row.id).unwrap();
    assert_eq!(recovered.status, MeetingStatus::Interrupted);
    assert_eq!(
        verify(&clone_store, &row.id, seconds * 100),
        (chunks, frames, bytes)
    );
    let clone_runtime =
        MeetingRuntime::new(clone_store.clone(), clone_root, services(factory)).unwrap();
    clone_runtime
        .export(&row.id, &root.join("recovered.wav"), "audio", "en")
        .unwrap();
    let mut wav = hound::WavReader::open(root.join("recovered.wav")).unwrap();
    assert_eq!(wav.spec().channels, 2);
    assert_eq!(wav.spec().sample_rate, 16000);
    assert_eq!(wav.spec().bits_per_sample, 16);
    assert_eq!(wav.duration(), (seconds * 16000) as u32);
    let mut sample_count = 0u64;
    let mut channel_energy = [0u64; 2];
    for sample in wav.samples::<i16>() {
        channel_energy[sample_count as usize % 2] += sample.unwrap().unsigned_abs() as u64;
        sample_count += 1;
    }
    assert_eq!(sample_count, seconds * 16000 * 2);
    assert!(channel_energy.iter().all(|energy| *energy > 0));
    drop(clone_store.edit(&row.id).unwrap()); // recovered audio can be claimed for explicit retry
    let result = serde_json::json!({"phase":"complete","wall_start_unix":wall_start,"wall_end_unix":SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs_f64(),"recording_elapsed_seconds":elapsed,"seconds":seconds,"id":row.id,"chunks":chunks,"frames":frames,"payload_bytes":bytes,"capture_complete":manifest.capture_complete,"memory":memory(),"stats":&*stats.lock().unwrap(),"packet_verification":true,"recovery_copy_verified":true,"wav_export_verified":true,"hardware_capture":false,"real_asr":false});
    fs::write(
        root.join("result.json"),
        serde_json::to_vec_pretty(&result).unwrap(),
    )
    .unwrap();
    println!("{result}");
}
