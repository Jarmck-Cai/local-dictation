use super::super::contracts::MeetingSegment;
use super::*;
use std::sync::atomic::AtomicUsize;

static NEXT: AtomicU64 = AtomicU64::new(0);
const START: u64 = 1_000_000;
const END: u64 = 1_200_000;

#[test]
#[cfg(windows)]
fn runtime_history_restore_keeps_other_activity_newer_than_every_history_row() {
    use std::os::windows::fs::OpenOptionsExt;
    for status in [
        MeetingStatus::Recording,
        MeetingStatus::Paused,
        MeetingStatus::Processing,
    ] {
        let f = Fixture::new();
        f.recorded("damaged");
        f.recorded("busy");
        let runtime = f.runtime(Mode::Good);
        let dir = f.root.join("damaged");
        let backup = dir.join(".manifest-123-12.backup");
        fs::rename(dir.join("manifest.json"), &backup).unwrap();
        let held = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&backup)
            .unwrap();
        runtime.list().unwrap();
        let (generation, _, completion) = runtime.reserve("busy", Activity::Processing).unwrap();
        let mut active = summary(&f.store.load("busy").unwrap(), Some(37));
        active.status = status;
        publish(&runtime.inner, generation, active, false);
        let before = runtime.current().unwrap().unwrap();
        drop(held);
        let rows = runtime.list().unwrap();
        let current = runtime.current().unwrap().unwrap();
        println!(
            "REFRESH_ACTIVITY_TRACE:{}",
            serde_json::json!({"before": before, "rows": rows})
        );
        let mut terminal = current.clone();
        terminal.status = MeetingStatus::Complete;
        finish_runtime(
            &runtime.inner,
            generation,
            Ok(terminal),
            MeetingStatus::Failed,
            &completion,
        );
        let busy = rows.iter().find(|v| v.id == "busy").unwrap();
        let history = rows.iter().find(|v| v.id == "damaged").unwrap();
        assert!(
            busy.revision > history.revision,
            "history must not clear another active job"
        );
        assert_eq!(
            serde_json::to_value(busy).unwrap(),
            serde_json::to_value(&current).unwrap()
        );
        assert_eq!(busy.status, status);
        assert_eq!(busy.progress_percent, Some(37));
    }
}

#[test]
#[cfg(windows)]
fn runtime_unlock_before_first_list_or_after_marker_never_revives_an_old_job() {
    use std::os::windows::fs::OpenOptionsExt;
    for status in [
        MeetingStatus::Recording,
        MeetingStatus::Paused,
        MeetingStatus::Processing,
    ] {
        for cached_marker in [false, true] {
            let f = Fixture::new();
            f.recorded("damaged");
            let mut m = f.store.load("damaged").unwrap();
            m.status = status;
            if status != MeetingStatus::Processing {
                m.capture_complete = None;
                m.chunks[0].finalized = false;
            }
            let dir = f.root.join("damaged");
            let backup = dir.join(".manifest-123-11.backup");
            fs::write(&backup, serde_json::to_vec(&m).unwrap()).unwrap();
            fs::remove_file(dir.join("manifest.json")).unwrap();
            let held = OpenOptions::new()
                .read(true)
                .share_mode(0)
                .open(&backup)
                .unwrap();
            let runtime = f.runtime(Mode::Good);
            if cached_marker {
                assert_eq!(runtime.list().unwrap()[0].status, MeetingStatus::Failed);
            }
            drop(held);
            let row = runtime.list().unwrap().remove(0);
            assert_eq!(
                row.status,
                if status == MeetingStatus::Processing {
                    MeetingStatus::Failed
                } else {
                    MeetingStatus::Interrupted
                },
                "{status:?}, cached={cached_marker}"
            );
            assert!(runtime.current().unwrap().is_none());
            assert_eq!(
                f.store.load("damaged").unwrap().status,
                status,
                "listing does not run recovery or restart a job"
            );
            runtime.transcribe("damaged").unwrap();
            wait_status(&runtime, MeetingStatus::Complete);
            assert_eq!(
                f.store.load("damaged").unwrap().capture_complete,
                Some(status == MeetingStatus::Processing)
            );
        }
    }
}

#[test]
#[cfg(windows)]
fn runtime_unlock_refresh_keeps_first_cause_and_does_not_revive_current_job() {
    use std::os::windows::fs::OpenOptionsExt;
    for (known_cause, old_status) in [false, true].into_iter().flat_map(|known| {
        [
            MeetingStatus::Recording,
            MeetingStatus::Paused,
            MeetingStatus::Processing,
        ]
        .into_iter()
        .map(move |status| (known, status))
    }) {
        let f = Fixture::new();
        f.recorded("damaged");
        let runtime = f.runtime(Mode::Good);
        let mut m = f.store.load("damaged").unwrap();
        m.status = old_status;
        m.capture_complete = if old_status == MeetingStatus::Processing {
            Some(true)
        } else {
            None
        };
        m.capture_error = None;
        m.chunks[0].finalized = old_status == MeetingStatus::Processing;
        let dir = f.root.join("damaged");
        fs::write(dir.join("manifest.json"), serde_json::to_vec(&m).unwrap()).unwrap();
        if known_cause {
            let (generation, _, done) = runtime.reserve("damaged", Activity::Recording).unwrap();
            let mut value = summary(&m, None);
            value.capture_error = Some("meeting_io".into());
            publish(&runtime.inner, generation, value, false);
            finish_runtime(
                &runtime.inner,
                generation,
                Err("meeting_atomic_replace_win32_1177".into()),
                MeetingStatus::Interrupted,
                &done,
            );
        }
        let backup = dir.join(".manifest-123-10.backup");
        fs::rename(dir.join("manifest.json"), &backup).unwrap();
        let held = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&backup)
            .unwrap();
        assert_eq!(runtime.list().unwrap()[0].status, MeetingStatus::Failed);
        drop(held);
        let row = runtime.list().unwrap().remove(0);
        let expected_status = if old_status == MeetingStatus::Processing {
            MeetingStatus::Failed
        } else {
            MeetingStatus::Interrupted
        };
        assert_eq!(row.status, expected_status);
        if known_cause {
            assert_eq!(runtime.current().unwrap().unwrap().status, expected_status);
        }
        assert_eq!(
            row.capture_error.as_deref(),
            if known_cause {
                Some("meeting_io")
            } else {
                None
            }
        );
        runtime.transcribe("damaged").unwrap();
        wait_status(&runtime, MeetingStatus::Complete);
        let stored = f.store.load("damaged").unwrap();
        assert_eq!(
            stored.capture_complete,
            Some(!known_cause && old_status == MeetingStatus::Processing)
        );
        if known_cause {
            assert_eq!(stored.capture_error.as_deref(), Some("meeting_io"));
        }
    }
}

#[test]
#[cfg(windows)]
fn runtime_missing_manifest_backup_restarts_and_preserves_other_meetings() {
    let f = Fixture::new();
    f.recorded("damaged");
    f.recorded("healthy");
    let dir = f.root.join("damaged");
    let old = fs::read(dir.join("manifest.json")).unwrap();
    fs::rename(
        dir.join("manifest.json"),
        dir.join(".manifest-123-8.backup"),
    )
    .unwrap();
    fs::write(
        dir.join(".manifest-123-8.tmp"),
        br#"{"id":"damaged","status":"complete"}"#,
    )
    .unwrap();
    let runtime = f.runtime(Mode::Good);
    assert_eq!(runtime.list().unwrap().len(), 2);
    assert_eq!(
        f.store.load("damaged").unwrap().status,
        MeetingStatus::Recorded
    );
    assert_eq!(fs::read(dir.join("manifest.json")).unwrap(), old);
    runtime.transcribe("damaged").unwrap();
    wait_status(&runtime, MeetingStatus::Complete);
    runtime.delete("damaged").unwrap();
    assert_eq!(runtime.list().unwrap().len(), 1);
}

#[test]
#[cfg(windows)]
fn runtime_blocked_backup_is_isolated_and_refreshes_after_unlock() {
    use std::os::windows::fs::OpenOptionsExt;
    let f = Fixture::new();
    f.recorded("damaged");
    f.recorded("healthy");
    let dir = f.root.join("damaged");
    let backup = dir.join(".manifest-123-9.backup");
    fs::rename(dir.join("manifest.json"), &backup).unwrap();
    fs::write(dir.join(".manifest-123-9.tmp"), b"unpublished").unwrap();
    let held = OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&backup)
        .unwrap();
    let runtime = f.runtime(Mode::Good);
    let rows = runtime.list().unwrap();
    assert_eq!(rows.len(), 2);
    let bad = rows.iter().find(|r| r.id == "damaged").unwrap();
    assert_eq!(bad.status, MeetingStatus::Failed);
    assert_eq!(
        bad.error.as_deref(),
        Some("meeting_manifest_restore_blocked")
    );
    assert!(bad.capture_error.is_none());
    assert!(runtime.delete("damaged").is_err());
    assert!(backup.exists());
    runtime.transcribe("healthy").unwrap();
    wait_status(&runtime, MeetingStatus::Complete);
    drop(held);
    let rows = runtime.list().unwrap();
    assert_eq!(
        rows.iter().find(|r| r.id == "damaged").unwrap().status,
        MeetingStatus::Recorded
    );
    runtime.transcribe("damaged").unwrap();
    wait_status(&runtime, MeetingStatus::Complete);
    assert!(runtime.current().unwrap().unwrap().capture_error.is_none());
}

#[test]
fn runtime_list_drops_records_expired_by_background_maintenance() {
    let f = Fixture::new();
    f.recorded("synthetic");
    let runtime = f.runtime(Mode::Good);
    assert_eq!(runtime.list().unwrap().len(), 1);
    let created = f.store.load("synthetic").unwrap().created_at_unix_secs;
    f.store.prune(created + 8 * 86400).unwrap();
    assert!(
        runtime.list().unwrap().is_empty(),
        "expired disk rows must not live forever in the snapshot cache"
    );
}

#[test]
#[cfg(windows)]
fn runtime_retry_persists_first_fault_after_recovery_was_unwritable() {
    use std::os::windows::fs::OpenOptionsExt;
    let f = Fixture::new();
    f.recorded("synthetic");
    let runtime = f.runtime(Mode::Good);
    let mut m = f.store.load("synthetic").unwrap();
    m.status = MeetingStatus::Recording;
    m.capture_complete = None;
    m.capture_error = None;
    m.chunks[0].finalized = false;
    fs::write(
        f.root.join("synthetic/manifest.json"),
        serde_json::to_vec(&m).unwrap(),
    )
    .unwrap();
    let (generation, _, completion) = runtime.reserve("synthetic", Activity::Recording).unwrap();
    let mut value = summary(&m, None);
    value.capture_error = Some("meeting_io".into());
    publish(&runtime.inner, generation, value, false);
    let held = OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(f.root.join("synthetic/manifest.json"))
        .unwrap();
    assert!(f
        .store
        .recover_capture_error("synthetic", Some("meeting_io".into()))
        .is_err());
    finish_runtime(
        &runtime.inner,
        generation,
        Err("meeting_atomic_replace_win32_32".into()),
        MeetingStatus::Interrupted,
        &completion,
    );
    assert_eq!(
        runtime.current().unwrap().unwrap().capture_error.as_deref(),
        Some("meeting_io")
    );
    drop(held);
    runtime
        .export(
            "synthetic",
            &f.directory.join("before-retry.wav"),
            "audio",
            "en",
        )
        .unwrap();
    runtime.transcribe("synthetic").unwrap();
    wait_status(&runtime, MeetingStatus::Complete);
    assert_eq!(
        runtime.current().unwrap().unwrap().capture_error.as_deref(),
        Some("meeting_io")
    );
    assert_eq!(
        f.store.load("synthetic").unwrap().capture_error.as_deref(),
        Some("meeting_io")
    );
}

#[test]
#[cfg(windows)]
fn runtime_first_fault_survives_failed_finish_then_real_recovery() {
    use std::os::windows::fs::OpenOptionsExt;
    let f = Fixture::new();
    let runtime = f.runtime(Mode::Good);
    let (generation, request, completion) =
        runtime.reserve("synthetic", Activity::Recording).unwrap();
    let mut writer = f
        .store
        .create("synthetic", "Synthetic", START, devices())
        .unwrap();
    writer.append(packet(Source::Microphone, 0)).unwrap();
    publish(
        &runtime.inner,
        generation,
        summary(writer.manifest(), None),
        false,
    );
    let held = OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(f.root.join("synthetic/manifest.json"))
        .unwrap();
    let (sender, receiver) = capture_event_queue();
    let mut invalid = packet(Source::Remote, 1);
    invalid.frames = 0;
    sender.try_send(CaptureEvent::Packet(invalid)).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let session = Box::new(FakeSession {
        mode: Mode::Good,
        sender,
        _stop: stop.clone(),
    });
    let result = record_actor(
        &runtime.inner,
        generation,
        writer,
        session,
        receiver,
        stop,
        request,
        Box::new(()),
    );
    assert_eq!(
        result.as_ref().err().map(String::as_str),
        Some("meeting_packet")
    );
    assert_eq!(
        runtime.current().unwrap().unwrap().capture_error.as_deref(),
        Some("meeting_packet")
    );
    drop(held);
    let recovered = f
        .store
        .recover_capture_error("synthetic", result.err())
        .unwrap();
    assert_eq!(recovered.capture_error.as_deref(), Some("meeting_packet"));
    assert_eq!(recovered.status, MeetingStatus::Interrupted);
    finish_runtime(
        &runtime.inner,
        generation,
        Ok(summary(&recovered, None)),
        MeetingStatus::Interrupted,
        &completion,
    );
    runtime.transcribe("synthetic").unwrap();
    wait_status(&runtime, MeetingStatus::Complete);
    assert_eq!(
        runtime.current().unwrap().unwrap().capture_error.as_deref(),
        Some("meeting_packet")
    );
}

#[test]
fn runtime_old_summary_json_accepts_missing_capture_error() {
    let f = Fixture::new();
    f.recorded("synthetic");
    let mut old = serde_json::to_value(summary(&f.store.load("synthetic").unwrap(), None)).unwrap();
    old.as_object_mut().unwrap().remove("capture_error");
    assert!(serde_json::from_value::<MeetingSummary>(old)
        .unwrap()
        .capture_error
        .is_none());
}

#[test]
fn runtime_late_list_keeps_concurrent_new_recording_and_newer_stop() {
    let f = Fixture::new();
    f.recorded("synthetic");
    let runtime = f.runtime(Mode::Good);
    let stale = f.store.list().unwrap();
    let rows = runtime
        .list_with(|| {
            let started = runtime.start("new", "mic", "remote").unwrap();
            runtime.stop(&started.id).unwrap();
            Ok(stale)
        })
        .unwrap();
    assert_eq!(rows.len(), 2);
    let current = runtime.current().unwrap().unwrap();
    assert!(rows.iter().any(|r| r.id == current.id
        && r.revision == current.revision
        && r.status == MeetingStatus::Recorded));
}

#[test]
fn runtime_list_io_releases_state_and_late_list_cannot_resurrect_deleted() {
    let f = Fixture::new();
    f.recorded("synthetic");
    let runtime = f.runtime(Mode::Good);
    let manifests = f.store.list().unwrap();
    let rows = runtime
        .list_with(|| {
            assert!(
                runtime.inner.state.try_lock().is_ok(),
                "disk IO must not hold runtime state"
            );
            runtime.delete("synthetic").unwrap();
            Ok(manifests)
        })
        .unwrap();
    assert!(rows.is_empty());
    assert!(runtime.list().unwrap().is_empty());
}

#[test]
fn runtime_summary_keeps_capture_cause_after_transcription() {
    let f = Fixture::new();
    f.recorded("synthetic");
    let mut m = f.store.load("synthetic").unwrap();
    m.capture_complete = Some(false);
    m.capture_error = Some("meeting_io".into());
    fs::write(
        f.root.join("synthetic/manifest.json"),
        serde_json::to_vec(&m).unwrap(),
    )
    .unwrap();
    let runtime = f.runtime(Mode::Good);
    assert_eq!(
        serde_json::to_value(&runtime.list().unwrap()[0]).unwrap()["capture_error"],
        "meeting_io"
    );
    runtime.transcribe("synthetic").unwrap();
    wait_status(&runtime, MeetingStatus::Complete);
    assert_eq!(
        serde_json::to_value(runtime.current().unwrap().unwrap()).unwrap()["capture_error"],
        "meeting_io"
    );
}

struct Fixture {
    directory: PathBuf,
    root: PathBuf,
    store: MeetingStore,
}
impl Fixture {
    fn new() -> Self {
        let directory = std::env::current_dir()
            .unwrap()
            .join("outputs")
            .join("meeting-runtime-fixtures")
            .join(format!(
                "{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        let root = directory.join("meetings");
        fs::create_dir_all(&root).unwrap();
        Self {
            store: MeetingStore::new(&root).unwrap(),
            directory,
            root,
        }
    }
    fn services(&self, mode: Mode) -> RuntimeServices {
        RuntimeServices {
            devices: Arc::new(|| Ok(devices())),
            capture: Arc::new(FakeFactory(mode)),
            acquire_recording: Arc::new(|| Ok(Box::new(()))),
            process: Arc::new(|_, id, _, progress| {
                progress(40);
                Ok(MeetingTranscript {
                    id: id.into(),
                    complete: true,
                    segments: vec![],
                    warnings: vec![],
                })
            }),
            emit: Arc::new(|_| {}),
        }
    }
    fn runtime(&self, mode: Mode) -> MeetingRuntime {
        MeetingRuntime::new(self.store.clone(), self.root.clone(), self.services(mode)).unwrap()
    }
    fn recorded(&self, id: &str) {
        let mut writer = self.store.create(id, "合成会议", START, devices()).unwrap();
        for source in [Source::Microphone, Source::Remote] {
            writer.heartbeat(source, 0, START).unwrap();
            writer.append(packet(source, 0)).unwrap();
            writer.append(packet(source, 1)).unwrap();
            writer.heartbeat(source, 0, END).unwrap();
        }
        writer
            .finish(MeetingStatus::Recorded, Some(END), None)
            .unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}
fn devices() -> Vec<DeviceInfo> {
    vec![
        DeviceInfo {
            id: "mic".into(),
            name: "Synthetic mic".into(),
            source: Source::Microphone,
            is_default: true,
        },
        DeviceInfo {
            id: "remote".into(),
            name: "Synthetic remote".into(),
            source: Source::Remote,
            is_default: true,
        },
    ]
}
fn packet(source: Source, sequence: u64) -> CapturePacket {
    let value = if source == Source::Microphone {
        0.25_f32
    } else {
        -0.5_f32
    };
    CapturePacket {
        source,
        epoch: 0,
        sequence,
        device_frame_position: sequence * 320,
        qpc_100ns: 900_000 + sequence * 200_000,
        frames: 320,
        flags: if sequence == 0 { 1 } else { 0 },
        format: AudioFormat {
            sample_rate: 16000,
            channels: 1,
            bits_per_sample: 32,
            valid_bits: 32,
            channel_mask: 4,
            encoding: AudioEncoding::Float,
        },
        data: (0..320).flat_map(|_| value.to_le_bytes()).collect(),
    }
}
#[derive(Clone, Copy)]
enum Mode {
    Good,
    LastSenderStops,
    SourceFault,
    QueueFull,
    InvalidPacket,
    SlowDrain,
}
struct FakeFactory(Mode);
struct FakeSession {
    mode: Mode,
    sender: CaptureSender,
    _stop: Arc<AtomicBool>,
}
impl CaptureFactory for FakeFactory {
    fn start(
        &self,
        config: CaptureConfig,
        sender: CaptureSender,
        stop: Arc<AtomicBool>,
    ) -> RuntimeResult<Box<dyn CaptureSession>> {
        assert_eq!(config.microphone_id, "mic");
        assert_eq!(config.render_id, "remote");
        for source in [Source::Microphone, Source::Remote] {
            sender
                .try_send(CaptureEvent::Heartbeat {
                    source,
                    qpc_100ns: START,
                })
                .unwrap();
            sender
                .try_send(CaptureEvent::Packet(packet(source, 0)))
                .unwrap();
        }
        match self.0 {
            Mode::SourceFault => {
                sender
                    .try_send(CaptureEvent::Error {
                        source: Source::Remote,
                        code: "meeting_capture_get_buffer_88890004".into(),
                    })
                    .unwrap();
                stop.store(true, Ordering::Release);
            }
            Mode::QueueFull => {
                while sender
                    .try_send(CaptureEvent::Heartbeat {
                        source: Source::Remote,
                        qpc_100ns: START,
                    })
                    .is_ok()
                {}
                stop.store(true, Ordering::Release);
            }
            Mode::InvalidPacket => {
                let mut invalid = packet(Source::Microphone, 1);
                invalid.frames = 0;
                sender.try_send(CaptureEvent::Packet(invalid)).unwrap();
            }
            _ => (),
        }
        Ok(Box::new(FakeSession {
            mode: self.0,
            sender,
            _stop: stop,
        }))
    }
}
impl CaptureSession for FakeSession {
    fn start_qpc_100ns(&self) -> u64 {
        START
    }
    fn stop(self: Box<Self>) -> RuntimeResult<u64> {
        if matches!(self.mode, Mode::LastSenderStops) {
            drop(self.sender);
            // Deterministically expose the gap between producer drop and the
            // capture-stop thread publishing its joined result.
            thread::sleep(Duration::from_millis(100));
            return Ok(END);
        }
        if matches!(self.mode, Mode::SlowDrain) {
            // More events than the complete queue: stop can only succeed while
            // the production writer actor is draining it concurrently.
            for _ in 0..4096 {
                loop {
                    match self.sender.try_send(CaptureEvent::Heartbeat {
                        source: Source::Remote,
                        qpc_100ns: START,
                    }) {
                        Ok(()) => break,
                        Err(mpsc::TrySendError::Full(_)) => thread::yield_now(),
                        Err(_) => return Err("meeting_capture_consumer_gone".into()),
                    }
                }
            }
        }
        for source in [Source::Microphone, Source::Remote] {
            loop {
                match self
                    .sender
                    .try_send(CaptureEvent::Packet(packet(source, 1)))
                {
                    Ok(()) => break,
                    Err(mpsc::TrySendError::Full(_)) => thread::yield_now(),
                    Err(_) => return Err("meeting_capture_consumer_gone".into()),
                }
            }
        }
        if matches!(self.mode, Mode::SourceFault) {
            return Err("meeting_capture_get_buffer_88890004".into());
        }
        if matches!(self.mode, Mode::QueueFull) {
            return Err("meeting_capture_queue_full".into());
        }
        for source in [Source::Microphone, Source::Remote] {
            loop {
                match self.sender.try_send(CaptureEvent::Heartbeat {
                    source,
                    qpc_100ns: END,
                }) {
                    Ok(()) => break,
                    Err(mpsc::TrySendError::Full(_)) => thread::yield_now(),
                    Err(_) => return Err("meeting_capture_consumer_gone".into()),
                }
            }
        }
        Ok(END)
    }
}

#[test]
fn runtime_normal_stop_last_sender_drop_is_not_capture_failure() {
    let f = Fixture::new();
    let runtime = f.runtime(Mode::LastSenderStops);
    let row = runtime.start("Synthetic", "mic", "remote").unwrap();
    let stopped = runtime.stop(&row.id).unwrap();
    assert_eq!(stopped.status, MeetingStatus::Recorded);
    assert_eq!(f.store.load(&row.id).unwrap().capture_complete, Some(true));
    assert!(stopped.capture_error.is_none());
}
fn wait_status(runtime: &MeetingRuntime, status: MeetingStatus) -> MeetingSummary {
    let began = Instant::now();
    loop {
        let value = runtime.current().unwrap().unwrap();
        if value.status == status {
            return value;
        }
        assert!(
            began.elapsed() < Duration::from_secs(5),
            "status {:?}, expected {:?}",
            value.status,
            status
        );
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn runtime_recording_uses_committed_boundaries_and_drains_while_stopping() {
    let f = Fixture::new();
    let runtime = f.runtime(Mode::SlowDrain);
    let started = runtime.start("合成会议", "mic", "remote").unwrap();
    assert_eq!(started.status, MeetingStatus::Recording);
    assert!(
        matches!(runtime.start("second", "mic", "remote"), Err(error) if error == "meeting_busy")
    );
    assert!(matches!(runtime.delete(&started.id), Err(error) if error == "meeting_active"));
    let stopped = runtime.stop(&started.id).unwrap();
    assert_eq!(stopped.status, MeetingStatus::Recorded);
    assert_eq!(stopped.duration_ms, 20);
    assert_eq!(
        runtime.stop(&started.id).unwrap().status,
        MeetingStatus::Recorded
    );
    let manifest = f.store.load(&started.id).unwrap();
    assert_eq!(manifest.start_qpc_100ns, START);
    assert_eq!(manifest.end_qpc_100ns, Some(END));
    assert_eq!(manifest.source_health.len(), 2);
    for health in &manifest.source_health {
        assert_eq!(
            (
                health.first_heartbeat_qpc_100ns,
                health.last_heartbeat_qpc_100ns
            ),
            (START, END)
        );
    }
    assert_eq!(
        manifest
            .chunks
            .iter()
            .map(|chunk| chunk.packets)
            .sum::<u64>(),
        4
    );
    assert!(manifest.chunks.iter().all(|chunk| chunk.finalized));
    assert!(runtime.transcript(&started.id).unwrap().is_none());
}

#[test]
fn runtime_source_failure_and_full_queue_interrupt_without_losing_valid_raw() {
    for (mode, code) in [
        (Mode::SourceFault, "meeting_capture_get_buffer_88890004"),
        (Mode::QueueFull, "meeting_capture_queue_full"),
        (Mode::InvalidPacket, "meeting_packet"),
    ] {
        let f = Fixture::new();
        let runtime = f.runtime(mode);
        let started = runtime.start("failure", "mic", "remote").unwrap();
        let stopped = wait_status(&runtime, MeetingStatus::Interrupted);
        assert_eq!(stopped.error.as_deref(), Some(code));
        let manifest = f.store.load(&started.id).unwrap();
        assert!(
            manifest
                .chunks
                .iter()
                .map(|chunk| chunk.packets)
                .sum::<u64>()
                >= 2
        );
        for chunk in &manifest.chunks {
            let mut reader = f.store.open_reader_for_chunk(&started.id, chunk).unwrap();
            while let Some(packet) = reader.next_packet().unwrap() {
                assert_eq!(packet.data.len(), 1280);
            }
        }
    }
}

#[test]
fn runtime_export_audio_is_exact_stereo_and_alignment_failure_preserves_old_file() {
    let f = Fixture::new();
    f.recorded("synthetic");
    let runtime = f.runtime(Mode::Good);
    let target = f.directory.join("meeting.wav");
    fs::write(&target, b"old file").unwrap();
    runtime.export("synthetic", &target, "audio", "en").unwrap();
    let mut wav = hound::WavReader::open(&target).unwrap();
    assert_eq!(wav.spec().channels, 2);
    assert_eq!(wav.spec().sample_rate, 16000);
    assert_eq!(wav.spec().bits_per_sample, 16);
    let samples: Vec<i16> = wav.samples::<i16>().map(Result::unwrap).collect();
    assert_eq!(samples.len(), 640);
    assert!((samples[300] as i32 - 8192).abs() <= 2);
    assert!((samples[301] as i32 + 16384).abs() <= 2);
    drop(wav);
    let original = fs::read(&target).unwrap();
    let mut broken = f
        .store
        .create("broken", "broken", START, devices())
        .unwrap();
    let mut invalid = packet(Source::Microphone, 0);
    invalid.flags = 4;
    broken.append(invalid).unwrap();
    broken
        .finish(MeetingStatus::Interrupted, Some(END), None)
        .unwrap();
    assert!(runtime.export("broken", &target, "audio", "en").is_err());
    assert_eq!(fs::read(&target).unwrap(), original);
    assert!(!fs::read_dir(&f.directory).unwrap().any(|entry| entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .contains("qingshu-export")));
}

#[test]
fn runtime_export_rejects_meeting_root_relative_and_unsafe_targets() {
    let f = Fixture::new();
    f.recorded("synthetic");
    let runtime = f.runtime(Mode::Good);
    assert!(runtime
        .export("synthetic", &f.root.join("export.wav"), "audio", "en")
        .is_err());
    assert!(runtime
        .export("synthetic", Path::new("relative.wav"), "audio", "en")
        .is_err());
    assert!(runtime
        .export(
            "synthetic",
            &f.directory.join("..\\escape.wav"),
            "audio",
            "en"
        )
        .is_err());
    assert!(runtime
        .export(
            "synthetic",
            &f.directory.join("alternate.txt:private"),
            "audio",
            "en"
        )
        .is_err());
}

struct FailingWriterFactory(PathBuf);
impl CaptureFactory for FailingWriterFactory {
    fn start(
        &self,
        _: CaptureConfig,
        sender: CaptureSender,
        stop: Arc<AtomicBool>,
    ) -> RuntimeResult<Box<dyn CaptureSession>> {
        sender
            .try_send(CaptureEvent::Heartbeat {
                source: Source::Microphone,
                qpc_100ns: START,
            })
            .unwrap();
        sender
            .try_send(CaptureEvent::Packet(packet(Source::Microphone, 0)))
            .unwrap();
        let source = sender.clone();
        let root = self.0.clone();
        thread::spawn(move || {
            let began = Instant::now();
            loop {
                let directory = fs::read_dir(&root).unwrap().find_map(|entry| {
                    let path = entry.ok()?.path();
                    path.join("chunk-00000000.bin").exists().then_some(path)
                });
                if let Some(directory) = directory {
                    // This is an actual OpenOptions/create_new disk failure on
                    // the second chunk, after valid microphone raw was saved.
                    fs::create_dir(directory.join("chunk-00000001.bin")).unwrap();
                    source
                        .try_send(CaptureEvent::Packet(packet(Source::Remote, 0)))
                        .unwrap();
                    break;
                }
                assert!(began.elapsed() < Duration::from_secs(2));
                thread::sleep(Duration::from_millis(2));
            }
        });
        Ok(Box::new(FakeSession {
            mode: Mode::Good,
            sender,
            _stop: stop,
        }))
    }
}

#[test]
fn runtime_real_chunk_open_failure_interrupts_and_retains_already_saved_raw() {
    let f = Fixture::new();
    let mut services = f.services(Mode::Good);
    services.capture = Arc::new(FailingWriterFactory(f.root.clone()));
    let runtime = MeetingRuntime::new(f.store.clone(), f.root.clone(), services).unwrap();
    let current = runtime.start("disk failure", "mic", "remote").unwrap();
    let terminal = wait_status(&runtime, MeetingStatus::Interrupted);
    assert_eq!(terminal.error.as_deref(), Some("meeting_io"));
    let manifest = f.store.load(&current.id).unwrap();
    assert_eq!(manifest.chunks.len(), 1);
    let mut raw = f
        .store
        .open_reader_for_chunk(&current.id, &manifest.chunks[0])
        .unwrap();
    assert_eq!(
        raw.next_packet().unwrap().unwrap().data,
        packet(Source::Microphone, 0).data
    );
    assert!(raw.next_packet().unwrap().is_none());
}

#[test]
#[ignore = "native link creation permission; run check-meeting-runtime.py --links"]
fn runtime_export_rejects_hardlink_target_without_altering_original() {
    let f = Fixture::new();
    f.recorded("synthetic");
    let runtime = f.runtime(Mode::Good);
    let old = f.directory.join("old.wav");
    fs::write(&old, b"old").unwrap();
    let linked = f.directory.join("linked.wav");
    fs::hard_link(&old, &linked).unwrap();
    assert!(
        matches!(runtime.export("synthetic", &linked, "audio", "en"), Err(error) if error == "meeting_link")
    );
    assert_eq!(fs::read(&old).unwrap(), b"old");
}

#[test]
#[ignore = "native link creation permission; run check-meeting-runtime.py --links"]
fn runtime_export_rejects_linked_parent_and_target_without_altering_original() {
    let f = Fixture::new();
    f.recorded("synthetic");
    let runtime = f.runtime(Mode::Good);
    let outside = f.directory.join("outside");
    fs::create_dir(&outside).unwrap();
    let original = outside.join("original.wav");
    fs::write(&original, b"original").unwrap();
    let link = f.directory.join("linked-parent");
    #[cfg(windows)]
    {
        let result = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&outside)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "junction fixture creation: {} {}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, &link).unwrap();
    assert!(
        matches!(runtime.export("synthetic", &link.join("original.wav"), "audio", "en"), Err(error) if error == "meeting_link")
    );
    assert_eq!(fs::read(original).unwrap(), b"original");
    assert_eq!(fs::read_dir(outside).unwrap().count(), 1);
}

#[test]
fn runtime_export_text_both_languages_and_failure_keep_existing_file() {
    let f = Fixture::new();
    f.recorded("synthetic");
    let transcript = MeetingTranscript {
        id: "synthetic".into(),
        complete: false,
        warnings: vec![],
        segments: vec![
            MeetingSegment {
                id: 0,
                source: Source::Microphone,
                start_ms: 0,
                end_ms: 10,
                text: "本地".into(),
            },
            MeetingSegment {
                id: 1,
                source: Source::Remote,
                start_ms: 10,
                end_ms: 20,
                text: "远端内容".into(),
            },
        ],
    };
    let mut writer = f.store.edit("synthetic").unwrap();
    writer
        .write_transcript(&serde_json::to_vec(&transcript).unwrap())
        .unwrap();
    drop(writer);
    let runtime = f.runtime(Mode::Good);
    let target = f.directory.join("meeting.txt");
    runtime.export("synthetic", &target, "text", "zh").unwrap();
    let chinese = fs::read_to_string(&target).unwrap();
    assert!(chinese.contains("麦克风：本地"));
    assert!(chinese.contains("电脑声音：远端内容"));
    runtime.export("synthetic", &target, "text", "en").unwrap();
    let english = fs::read_to_string(&target).unwrap();
    assert!(english.contains("Microphone: 本地"));
    assert!(english.contains("Computer audio: 远端内容"));
    f.recorded("missing");
    assert!(runtime.export("missing", &target, "text", "en").is_err());
    assert_eq!(fs::read_to_string(&target).unwrap(), english);
}

#[test]
fn runtime_processing_error_panic_and_invalid_id_publish_terminal_static_errors() {
    for mode in 0..3 {
        let f = Fixture::new();
        f.recorded("synthetic");
        let mut services = f.services(Mode::Good);
        services.process = Arc::new(move |_, id, _, _| match mode {
            0 => Err("private device transcript path".into()),
            1 => panic!("synthetic worker panic"),
            _ => Ok(MeetingTranscript {
                id: format!("{id}-wrong"),
                complete: true,
                segments: vec![],
                warnings: vec![],
            }),
        });
        let runtime = MeetingRuntime::new(f.store.clone(), f.root.clone(), services).unwrap();
        runtime.transcribe("synthetic").unwrap();
        let terminal = wait_status(&runtime, MeetingStatus::Failed);
        assert_eq!(
            terminal.error.as_deref(),
            Some(match mode {
                0 => "meeting_internal",
                1 => "meeting_processing_panicked",
                _ => "meeting_transcript",
            })
        );
        assert!(f.store.edit("synthetic").is_ok());
    }
}

#[test]
fn runtime_failed_final_manifest_io_still_publishes_readable_failure() {
    let f = Fixture::new();
    f.recorded("synthetic");
    let mut services = f.services(Mode::Good);
    let manifest = f.root.join("synthetic/manifest.json");
    services.process = Arc::new(move |_, _, _, _| {
        fs::remove_file(&manifest).unwrap();
        fs::create_dir(&manifest).unwrap();
        Err("meeting_io".into())
    });
    let runtime = MeetingRuntime::new(f.store.clone(), f.root.clone(), services).unwrap();
    runtime.transcribe("synthetic").unwrap();
    let terminal = wait_status(&runtime, MeetingStatus::Failed);
    assert_eq!(terminal.error.as_deref(), Some("meeting_path"));
    assert!(runtime.inner.state.lock().unwrap().active.is_none());
}

#[test]
fn runtime_record_lease_is_not_held_by_processing_and_old_completion_cannot_replace_next() {
    let f = Fixture::new();
    f.recorded("synthetic");
    let (finish_tx, finish_rx) = mpsc::channel();
    let finish_rx = Mutex::new(finish_rx);
    let mut services = f.services(Mode::Good);
    services.process = Arc::new(move |_, id, _, progress| {
        progress(70);
        finish_rx
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        Ok(MeetingTranscript {
            id: id.into(),
            complete: true,
            segments: vec![],
            warnings: vec![],
        })
    });
    let runtime = MeetingRuntime::new(f.store.clone(), f.root.clone(), services).unwrap();
    runtime.transcribe("synthetic").unwrap();
    assert!(matches!(runtime.stop("synthetic"), Err(error) if error == "meeting_busy"));
    assert!(
        matches!(runtime.export("synthetic", &f.directory.join("busy.wav"), "audio", "en"), Err(error) if error == "meeting_active")
    );
    finish_tx.send(()).unwrap();
    wait_status(&runtime, MeetingStatus::Complete);
    let next = runtime.start("new", "mic", "remote").unwrap();
    thread::sleep(Duration::from_millis(30));
    assert_eq!(runtime.current().unwrap().unwrap().id, next.id);
    runtime.stop(&next.id).unwrap();
}

#[test]
fn runtime_recording_keeps_idle_source_health_checkpointed_and_events_can_read_current() {
    let f = Fixture::new();
    let inner_slot: Arc<Mutex<Option<std::sync::Weak<Inner>>>> = Arc::new(Mutex::new(None));
    let slot = inner_slot.clone();
    let mut services = f.services(Mode::Good);
    services.emit = Arc::new(move |_| {
        if let Some(inner) = slot
            .lock()
            .unwrap()
            .as_ref()
            .and_then(std::sync::Weak::upgrade)
        {
            assert!(
                inner.state.try_lock().is_ok(),
                "events must be emitted outside the state mutex"
            );
        }
    });
    let runtime = MeetingRuntime::new(f.store.clone(), f.root.clone(), services).unwrap();
    *inner_slot.lock().unwrap() = Some(Arc::downgrade(&runtime.inner));
    let current = runtime.start("checkpoint", "mic", "remote").unwrap();
    thread::sleep(Duration::from_millis(1200));
    let manifest = f.store.load(&current.id).unwrap();
    assert_eq!(manifest.source_health.len(), 2);
    assert_eq!(
        manifest
            .chunks
            .iter()
            .map(|chunk| chunk.packets)
            .sum::<u64>(),
        2
    );
    runtime.stop(&current.id).unwrap();
}

#[test]
fn runtime_delayed_failed_event_keeps_older_revision_than_same_id_retry() {
    let f = Fixture::new();
    f.recorded("synthetic");
    let (blocked_tx, blocked_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let release_rx = Mutex::new(release_rx);
    let (job_tx, job_rx) = mpsc::channel();
    let job_rx = Mutex::new(job_rx);
    let calls = AtomicUsize::new(0);
    let delivered = Arc::new(Mutex::new(Vec::<MeetingSummary>::new()));
    let delivered_sink = delivered.clone();
    let mut services = f.services(Mode::Good);
    services.emit = Arc::new(move |value| {
        if value.status == MeetingStatus::Failed {
            blocked_tx.send(()).unwrap();
            release_rx
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(3))
                .unwrap();
        }
        delivered_sink.lock().unwrap().push(value);
    });
    services.process = Arc::new(move |_, id, _, _| {
        if calls.fetch_add(1, Ordering::AcqRel) == 0 {
            return Err("meeting_asr".into());
        }
        job_rx
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(3))
            .unwrap();
        Ok(MeetingTranscript {
            id: id.into(),
            complete: true,
            segments: vec![],
            warnings: vec![],
        })
    });
    let runtime = MeetingRuntime::new(f.store.clone(), f.root.clone(), services).unwrap();
    let first = runtime.transcribe("synthetic").unwrap();
    blocked_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let retry = runtime.transcribe("synthetic").unwrap();
    assert!(retry.revision > first.revision);
    assert_eq!(runtime.current().unwrap().unwrap().revision, retry.revision);
    assert_eq!(runtime.list().unwrap()[0].revision, retry.revision);
    release_tx.send(()).unwrap();
    let began = Instant::now();
    loop {
        let old = delivered
            .lock()
            .unwrap()
            .iter()
            .find(|value| value.status == MeetingStatus::Failed)
            .cloned();
        if let Some(old) = old {
            assert!(
                old.revision < retry.revision,
                "delayed old Failed must be discarded by revision"
            );
            break;
        }
        assert!(began.elapsed() < Duration::from_secs(2));
        thread::yield_now();
    }
    job_tx.send(()).unwrap();
    let complete = wait_status(&runtime, MeetingStatus::Complete);
    assert!(complete.revision > retry.revision);
    let next = runtime.start("next", "mic", "remote").unwrap();
    let old = runtime
        .list()
        .unwrap()
        .into_iter()
        .find(|value| value.id == "synthetic")
        .unwrap();
    assert_eq!(
        old.revision, complete.revision,
        "list must retain revisions after another meeting becomes current"
    );
    let stopped = runtime.stop(&next.id).unwrap();
    assert_eq!(
        stopped.revision,
        runtime.current().unwrap().unwrap().revision
    );
}

#[test]
fn runtime_delayed_same_job_progress_keeps_revision_and_current_progress_monotonic() {
    let f = Fixture::new();
    f.recorded("synthetic");
    let (blocked_tx, blocked_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let release_rx = Mutex::new(release_rx);
    let delivered = Arc::new(Mutex::new(Vec::<MeetingSummary>::new()));
    let delivered_sink = delivered.clone();
    let mut services = f.services(Mode::Good);
    services.emit = Arc::new(move |value| {
        if value.progress_percent == Some(20) {
            blocked_tx.send(()).unwrap();
            release_rx
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(3))
                .unwrap();
        }
        delivered_sink.lock().unwrap().push(value);
    });
    let runtime = MeetingRuntime::new(f.store.clone(), f.root.clone(), services).unwrap();
    let (generation, _, completion) = runtime.reserve("synthetic", Activity::Processing).unwrap();
    let mut value = summary(&f.store.load("synthetic").unwrap(), Some(0));
    value.status = MeetingStatus::Processing;
    publish(&runtime.inner, generation, value, false);
    let inner = runtime.inner.clone();
    let old = thread::spawn(move || publish_progress(&inner, generation, 20));
    blocked_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    publish_progress(&runtime.inner, generation, 70);
    let newer = runtime.current().unwrap().unwrap();
    release_tx.send(()).unwrap();
    old.join().unwrap();
    let events = delivered.lock().unwrap();
    let old = events
        .iter()
        .find(|value| value.progress_percent == Some(20))
        .unwrap();
    let old_revision = old.revision;
    drop(events);
    publish_progress(&runtime.inner, generation, 10);
    let final_progress = runtime.current().unwrap().unwrap().progress_percent;
    let mut final_value = runtime.current().unwrap().unwrap();
    final_value.status = MeetingStatus::Complete;
    finish_runtime(
        &runtime.inner,
        generation,
        Ok(final_value),
        MeetingStatus::Failed,
        &completion,
    );
    assert_eq!(newer.progress_percent, Some(70));
    assert!(old_revision < newer.revision);
    assert_eq!(final_progress, Some(70));
}

#[test]
fn runtime_same_process_transcribe_recovers_nonfinalized_partial_tail_only() {
    let f = Fixture::new();
    f.recorded("synthetic");
    let mut manifest = f.store.load("synthetic").unwrap();
    let chunk_path = f.root.join("synthetic").join(&manifest.chunks[0].file);
    let committed = manifest.chunks[0].committed_bytes;
    let mut raw = OpenOptions::new().append(true).open(&chunk_path).unwrap();
    raw.write_all(&[0x55; 17]).unwrap();
    drop(raw);
    manifest.chunks[0].finalized = false;
    manifest.status = MeetingStatus::Interrupted;
    manifest.error = Some("meeting_io".into());
    manifest.capture_error = Some("meeting_io".into());
    fs::write(
        f.root.join("synthetic/manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let mut services = f.services(Mode::Good);
    services.process = Arc::new(move |store, id, _, _| {
        let manifest = store.load(id).map_err(str::to_owned)?;
        assert!(manifest.chunks.iter().all(|chunk| chunk.finalized));
        assert_eq!(manifest.capture_error.as_deref(), Some("meeting_io"));
        for chunk in &manifest.chunks {
            store
                .open_reader_for_chunk(id, chunk)
                .map_err(str::to_owned)?;
        }
        Ok(MeetingTranscript {
            id: id.into(),
            complete: true,
            segments: vec![],
            warnings: vec![],
        })
    });
    // Construct before injecting damage, so this exercises same-run retry rather
    // than only the startup recovery path.
    let restored = manifest.clone();
    manifest.chunks[0].finalized = true;
    fs::write(
        f.root.join("synthetic/manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let runtime = MeetingRuntime::new(f.store.clone(), f.root.clone(), services).unwrap();
    fs::write(
        f.root.join("synthetic/manifest.json"),
        serde_json::to_vec(&restored).unwrap(),
    )
    .unwrap();
    assert_eq!(fs::metadata(&chunk_path).unwrap().len(), committed + 17);
    runtime.transcribe("synthetic").unwrap();
    wait_status(&runtime, MeetingStatus::Complete);
    assert_eq!(fs::metadata(&chunk_path).unwrap().len(), committed);
    assert_eq!(
        f.store.load("synthetic").unwrap().capture_error.as_deref(),
        Some("meeting_io")
    );
}

#[test]
#[ignore = "injected partial write; run check-meeting-runtime.py --partial-write"]
fn runtime_partial_packet_write_is_salvaged_before_export_and_same_run_retry() {
    let f = Fixture::new();
    let runtime = f.runtime(Mode::Good);
    let started = runtime
        .start("synthetic partial write", "mic", "remote")
        .unwrap();
    let terminal = runtime.stop(&started.id).unwrap();
    assert_eq!(terminal.status, MeetingStatus::Interrupted);
    assert_eq!(terminal.error.as_deref(), Some("meeting_io"));
    let manifest = f.store.load(&started.id).unwrap();
    assert!(manifest.chunks.iter().all(|chunk| chunk.finalized));
    assert_eq!(manifest.capture_error.as_deref(), Some("meeting_io"));
    for chunk in &manifest.chunks {
        assert_eq!(
            fs::metadata(f.root.join(&started.id).join(&chunk.file))
                .unwrap()
                .len(),
            chunk.committed_bytes
        );
        f.store.open_reader_for_chunk(&started.id, chunk).unwrap();
    }
    runtime
        .export(
            &started.id,
            &f.directory.join("salvaged.wav"),
            "audio",
            "en",
        )
        .unwrap();
    runtime.transcribe(&started.id).unwrap();
    wait_status(&runtime, MeetingStatus::Complete);
    assert_eq!(
        f.store.load(&started.id).unwrap().capture_error.as_deref(),
        Some("meeting_io")
    );
}

#[test]
#[cfg(windows)]
fn runtime_recovery_io_failure_keeps_partial_raw_and_retries_after_file_unlock() {
    use std::os::windows::fs::OpenOptionsExt;
    let f = Fixture::new();
    f.recorded("synthetic");
    let runtime = f.runtime(Mode::Good);
    let mut manifest = f.store.load("synthetic").unwrap();
    let chunk_path = f.root.join("synthetic").join(&manifest.chunks[0].file);
    let committed = manifest.chunks[0].committed_bytes;
    let mut raw = OpenOptions::new().append(true).open(&chunk_path).unwrap();
    raw.write_all(&[0x55; 17]).unwrap();
    drop(raw);
    manifest.chunks[0].finalized = false;
    manifest.status = MeetingStatus::Interrupted;
    manifest.error = Some("meeting_io".into());
    manifest.capture_error = Some("meeting_io".into());
    fs::write(
        f.root.join("synthetic/manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let lock = OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&chunk_path)
        .unwrap();
    assert!(runtime.transcribe("synthetic").is_err());
    assert_eq!(lock.metadata().unwrap().len(), committed + 17);
    assert!(!f.store.load("synthetic").unwrap().chunks[0].finalized);
    assert!(runtime.inner.state.lock().unwrap().active.is_none());
    drop(lock);
    runtime.transcribe("synthetic").unwrap();
    wait_status(&runtime, MeetingStatus::Complete);
    assert_eq!(fs::metadata(chunk_path).unwrap().len(), committed);
}

#[test]
fn runtime_stale_processing_does_not_recover_healthy_raw_or_invent_capture_failure() {
    let f = Fixture::new();
    f.recorded("synthetic");
    let mut manifest = f.store.load("synthetic").unwrap();
    manifest.status = MeetingStatus::Processing;
    fs::write(
        f.root.join("synthetic/manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let original: Vec<Vec<u8>> = manifest
        .chunks
        .iter()
        .map(|chunk| fs::read(f.root.join("synthetic").join(&chunk.file)).unwrap())
        .collect();
    let runtime = f.runtime(Mode::Good);
    let after = f.store.load("synthetic").unwrap();
    assert_eq!(after.status, MeetingStatus::Failed);
    assert_eq!(
        after.error.as_deref(),
        Some("meeting_processing_interrupted")
    );
    assert!(after.capture_error.is_none());
    assert_eq!(after.end_qpc_100ns, manifest.end_qpc_100ns);
    for (chunk, bytes) in after.chunks.iter().zip(original) {
        assert_eq!(
            fs::read(f.root.join("synthetic").join(&chunk.file)).unwrap(),
            bytes
        );
    }
    assert_eq!(runtime.list().unwrap()[0].status, MeetingStatus::Failed);
}

#[test]
fn runtime_new_preserves_complete_and_recovers_only_stale_sessions() {
    let f = Fixture::new();
    f.recorded("complete");
    f.recorded("stale");
    f.store
        .edit("complete")
        .unwrap()
        .finish(MeetingStatus::Complete, Some(END), None)
        .unwrap();
    f.store
        .edit("stale")
        .unwrap()
        .finish(MeetingStatus::Processing, Some(END), None)
        .unwrap();
    let runtime = f.runtime(Mode::Good);
    assert_eq!(
        f.store.load("complete").unwrap().status,
        MeetingStatus::Complete
    );
    assert_eq!(f.store.load("stale").unwrap().status, MeetingStatus::Failed);
    assert_eq!(runtime.list().unwrap().len(), 2);
}

#[test]
fn runtime_processing_is_background_cancelled_with_lease_and_can_record_afterwards() {
    let f = Fixture::new();
    f.recorded("synthetic");
    let leases = Arc::new(AtomicUsize::new(0));
    struct Lease(Arc<AtomicUsize>);
    impl Drop for Lease {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::AcqRel);
        }
    }
    let (began_tx, began_rx) = mpsc::channel();
    let leases2 = leases.clone();
    let mut services = f.services(Mode::Good);
    services.acquire_recording = Arc::new(move || {
        leases2.fetch_add(1, Ordering::AcqRel);
        Ok(Box::new(Lease(leases2.clone())))
    });
    services.process = Arc::new(move |store, id, cancel, progress| {
        let mut writer = store.edit(id).map_err(str::to_owned)?;
        writer
            .checkpoint(MeetingStatus::Processing, Some(END), None)
            .map_err(str::to_owned)?;
        progress(25);
        began_tx.send(()).unwrap();
        while !cancel.load(Ordering::Acquire) {
            thread::sleep(Duration::from_millis(5));
        }
        let transcript = MeetingTranscript {
            id: id.into(),
            complete: false,
            segments: vec![MeetingSegment {
                id: 0,
                source: Source::Remote,
                start_ms: 0,
                end_ms: 20,
                text: "partial".into(),
            }],
            warnings: vec![],
        };
        writer
            .write_transcript(&serde_json::to_vec(&transcript).unwrap())
            .map_err(str::to_owned)?;
        writer
            .finish(MeetingStatus::Cancelled, Some(END), None)
            .map_err(str::to_owned)?;
        Ok(transcript)
    });
    let runtime = MeetingRuntime::new(f.store.clone(), f.root.clone(), services).unwrap();
    assert_eq!(
        runtime.transcribe("synthetic").unwrap().status,
        MeetingStatus::Processing
    );
    began_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(leases.load(Ordering::Acquire), 0);
    assert_eq!(
        runtime.current().unwrap().unwrap().progress_percent,
        Some(25)
    );
    assert!(matches!(runtime.delete("synthetic"), Err(error) if error == "meeting_active"));
    assert_eq!(f.store.delete("synthetic"), Err("meeting_active"));
    let created = f.store.load("synthetic").unwrap().created_at_unix_secs;
    assert!(f.store.prune(created + 8 * 86400).unwrap().is_empty());
    assert!(
        matches!(runtime.start("busy", "mic", "remote"), Err(error) if error == "meeting_busy")
    );
    runtime.cancel_transcription("synthetic").unwrap();
    wait_status(&runtime, MeetingStatus::Cancelled);
    assert!(!runtime.transcript("synthetic").unwrap().unwrap().complete);
    let recording = runtime.start("next", "mic", "remote").unwrap();
    assert_eq!(leases.load(Ordering::Acquire), 1);
    runtime.stop(&recording.id).unwrap();
    assert_eq!(leases.load(Ordering::Acquire), 0);
    assert_eq!(runtime.current().unwrap().unwrap().id, recording.id);
}

#[test]
fn runtime_drop_finishes_recording_and_explicit_missing_devices_do_not_fallback() {
    let f = Fixture::new();
    let runtime = f.runtime(Mode::Good);
    assert!(
        matches!(runtime.start("bad", "missing", "remote"), Err(error) if error == "meeting_device_unavailable")
    );
    let recording = runtime.start("shutdown", "mic", "remote").unwrap();
    drop(runtime);
    assert_eq!(
        f.store.load(&recording.id).unwrap().status,
        MeetingStatus::Recorded
    );
    assert!(f.store.edit(&recording.id).is_ok());
}

#[test]
fn runtime_exports_localized_labels_with_exact_timestamps() {
    let transcript = MeetingTranscript {
        id: "test".into(),
        complete: true,
        warnings: vec![],
        segments: vec![super::super::contracts::MeetingSegment {
            id: 0,
            source: Source::Microphone,
            start_ms: 1234,
            end_ms: 5678,
            text: "你好".into(),
        }],
    };
    let mut bytes = vec![];
    write_text(&mut bytes, &transcript, "zh").unwrap();
    assert_eq!(
        String::from_utf8(bytes).unwrap(),
        "[00:00:01.234 --> 00:00:05.678] 麦克风：你好\n"
    );
}

#[test]
fn beta_export_carries_source_limit_and_partial_transcript_notice() {
    let transcript = MeetingTranscript {
        id: "synthetic-beta".into(),
        complete: false,
        warnings: vec!["meeting_attribution_experimental".into()],
        segments: vec![],
    };
    for (language, source_limit, incomplete) in [
        ("zh", "不是说话人身份", "转录未完成"),
        ("en", "not speaker identities", "Incomplete transcript"),
    ] {
        let mut bytes = vec![];
        write_text(&mut bytes, &transcript, language).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("Beta"));
        assert!(text.contains(source_limit));
        assert!(text.contains(incomplete));
    }
}

#[test]
fn beta_export_marks_interrupted_recording_after_transcription_completes() {
    let transcript = MeetingTranscript {
        id: "synthetic-beta-interrupted".into(),
        complete: true,
        warnings: vec!["meeting_alignment_incomplete_recording".into()],
        segments: vec![],
    };
    for (language, message) in [("zh", "录音有中断"), ("en", "Recording was interrupted")] {
        let mut bytes = vec![];
        write_text(&mut bytes, &transcript, language).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains(message));
    }
}
