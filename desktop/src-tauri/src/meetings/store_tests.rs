use super::super::queue::capture_event_queue;
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

#[test]
#[cfg(windows)]
fn failed_transcript_replace_retains_staged_copy() {
    use std::os::windows::fs::OpenOptionsExt;
    let f = Fixture::new();
    let mut writer = f.writer();
    let original = br#"{"id":"synthetic","complete":false}"#;
    writer.write_transcript(original).unwrap();
    let path = f.root.join("synthetic/transcript.json");
    let held = OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&path)
        .unwrap();
    assert!(writer
        .write_transcript(br#"{"id":"synthetic","complete":true}"#)
        .is_err());
    assert_eq!(fs::read(path).unwrap(), original);
    assert!(fs::read_dir(f.root.join("synthetic")).unwrap().any(|e| e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".transcript-")));
    drop(held);
}

#[test]
fn retained_atomic_backups_do_not_block_explicit_delete_or_expiry() {
    for expire in [false, true] {
        let f = Fixture::new();
        f.writer()
            .finish(MeetingStatus::Recorded, None, None)
            .unwrap();
        for name in [".manifest-123-4.backup", ".transcript-123-5.backup"] {
            fs::write(f.root.join("synthetic").join(name), b"synthetic backup").unwrap();
        }
        if expire {
            let created = f.store.load("synthetic").unwrap().created_at_unix_secs;
            assert_eq!(
                f.store.prune(created + RETENTION_SECS).unwrap(),
                vec!["synthetic"]
            );
        } else {
            f.store.delete("synthetic").unwrap();
        }
        assert!(!f.root.join("synthetic").exists());
    }
}

#[test]
fn recovery_keeps_legacy_saved_first_cause_over_later_actor_error() {
    let f = Fixture::new();
    let mut writer = f.writer();
    writer.append(packet(0)).unwrap();
    drop(writer);
    let path = f.root.join("synthetic/manifest.json");
    let mut m = f.store.load("synthetic").unwrap();
    m.status = MeetingStatus::Interrupted;
    m.error = Some("meeting_io".into());
    m.capture_error = None;
    m.capture_complete = None;
    fs::write(path, serde_json::to_vec(&m).unwrap()).unwrap();
    let recovered = f
        .store
        .recover_capture_error("synthetic", Some("meeting_atomic_replace".into()))
        .unwrap();
    assert_eq!(recovered.capture_error.as_deref(), Some("meeting_io"));
}

#[test]
#[cfg(windows)]
fn state_io_default_reader_does_not_block_checkpoint() {
    let f = Fixture::new();
    let mut writer = f.writer();
    writer.append(packet(0)).unwrap();
    let path = f.root.join("synthetic/manifest.json");
    let mut reader = File::open(&path).unwrap();
    writer
        .checkpoint(MeetingStatus::Paused, None, None)
        .unwrap();
    assert_eq!(
        f.store.load("synthetic").unwrap().status,
        MeetingStatus::Paused
    );
    let mut old = String::new();
    reader.read_to_string(&mut old).unwrap();
    assert!(old.contains("recording"));
}

#[test]
fn state_io_production_load_concurrent_with_one_hundred_commits() {
    use std::sync::{atomic::AtomicBool, Arc, Barrier};
    let f = Fixture::new();
    let mut writer = f.writer();
    let stop = Arc::new(AtomicBool::new(false));
    let barrier = Arc::new(Barrier::new(2));
    let store = f.store.clone();
    let signal = stop.clone();
    let gate = barrier.clone();
    let worker = std::thread::spawn(move || {
        gate.wait();
        let mut reads = 0;
        while !signal.load(Ordering::Acquire) {
            store.load("synthetic").unwrap();
            reads += 1;
        }
        reads
    });
    barrier.wait();
    let result = (|| {
        for sequence in 0..100 {
            writer.append(packet(sequence))?;
            writer.checkpoint(MeetingStatus::Recording, None, None)?;
        }
        Ok::<_, &'static str>(())
    })();
    stop.store(true, Ordering::Release);
    assert!(worker.join().unwrap() > 0);
    result.unwrap();
    assert_eq!(
        writer
            .finish(MeetingStatus::Recorded, Some(200000), None)
            .unwrap()
            .chunks[0]
            .packets,
        100
    );
}

#[test]
fn legacy_interruption_is_upgraded_before_processing_changes_its_status() {
    let f = Fixture::new();
    let mut writer = f.writer();
    writer.append(packet(0)).unwrap();
    writer
        .finish(MeetingStatus::Recorded, Some(1350), None)
        .unwrap();
    let path = f.root.join("synthetic/manifest.json");
    let mut old: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    old.as_object_mut().unwrap().remove("capture_error");
    old.as_object_mut().unwrap().remove("capture_complete");
    old["status"] = serde_json::json!("interrupted");
    old["error"] = serde_json::json!("meeting_legacy_capture_failed");
    fs::write(path, serde_json::to_vec(&old).unwrap()).unwrap();
    let editor = f.store.edit("synthetic").unwrap();
    assert_eq!(editor.manifest().capture_complete, Some(false));
    assert_eq!(
        editor.manifest().capture_error.as_deref(),
        Some("meeting_legacy_capture_failed")
    );
    editor
        .finish(MeetingStatus::Complete, Some(1350), None)
        .unwrap();
    let retried = f.store.edit("synthetic").unwrap();
    assert_eq!(retried.manifest().capture_complete, Some(false));
    assert_eq!(
        retried.manifest().capture_error.as_deref(),
        Some("meeting_legacy_capture_failed")
    );
}

#[test]
fn recovering_only_a_stale_processing_job_keeps_capture_complete() {
    let f = Fixture::new();
    let mut writer = f.writer();
    writer.append(packet(0)).unwrap();
    writer
        .finish(MeetingStatus::Recorded, Some(1350), None)
        .unwrap();
    f.store
        .edit("synthetic")
        .unwrap()
        .finish(MeetingStatus::Processing, Some(1350), None)
        .unwrap();
    let recovered = f.store.recover("synthetic").unwrap();
    assert_eq!(recovered.capture_complete, Some(true));
    assert!(recovered.capture_error.is_none());
    let editor = f.store.edit("synthetic").unwrap();
    assert_eq!(editor.manifest().capture_complete, Some(true));
    assert!(editor.manifest().capture_error.is_none());
}

#[test]
fn recovery_marks_lost_committed_audio_even_after_a_healthy_stop() {
    let f = Fixture::new();
    let mut writer = f.writer();
    writer.append(packet(0)).unwrap();
    writer.append(packet(1)).unwrap();
    let complete = writer
        .finish(MeetingStatus::Recorded, Some(2600), None)
        .unwrap();
    let chunk = &complete.chunks[0];
    OpenOptions::new()
        .write(true)
        .open(f.root.join("synthetic").join(&chunk.file))
        .unwrap()
        .set_len(chunk.committed_bytes - 1)
        .unwrap();
    let recovered = f.store.recover("synthetic").unwrap();
    assert_eq!(recovered.capture_complete, Some(false));
    assert_eq!(
        recovered.capture_error.as_deref(),
        Some("meeting_capture_recovered")
    );
    assert_eq!(recovered.chunks[0].packets, 1);
}

#[test]
fn interrupted_partial_write_stays_recoverable_after_finish() {
    let f = Fixture::new();
    let mut writer = f.writer();
    writer.append(packet(0)).unwrap();
    let (index, file) = writer.open[0].as_mut().unwrap();
    let committed = writer.manifest.chunks[*index].committed_bytes;
    // Emulate a filesystem accepting part of a packet before returning I/O failure.
    file.write_all(&[7; 17]).unwrap();
    writer.failed = true;
    let stopped = writer
        .finish(
            MeetingStatus::Interrupted,
            Some(1350),
            Some("meeting_io".into()),
        )
        .unwrap();
    assert!(!stopped.chunks[0].finalized);
    let recovered = f.store.recover("synthetic").unwrap();
    assert_eq!(recovered.chunks[0].committed_bytes, committed);
    assert_eq!(
        fs::metadata(f.root.join("synthetic").join(&recovered.chunks[0].file))
            .unwrap()
            .len(),
        committed
    );
}

#[test]
fn capture_failure_provenance_survives_processing_and_retry_statuses() {
    let f = Fixture::new();
    let mut writer = f.writer();
    writer.append(packet(0)).unwrap();
    writer
        .finish(
            MeetingStatus::Interrupted,
            Some(1350),
            Some("meeting_capture_device_inactive".into()),
        )
        .unwrap();
    let mut processing = f.store.edit("synthetic").unwrap();
    processing
        .checkpoint(MeetingStatus::Processing, Some(1350), None)
        .unwrap();
    let completed = processing
        .finish(MeetingStatus::Complete, Some(1350), None)
        .unwrap();
    assert_eq!(
        completed.capture_error.as_deref(),
        Some("meeting_capture_device_inactive")
    );
    let retried = f
        .store
        .edit("synthetic")
        .unwrap()
        .finish(
            MeetingStatus::Failed,
            Some(1350),
            Some("meeting_transcription_failed".into()),
        )
        .unwrap();
    assert_eq!(retried.capture_error, completed.capture_error);
    assert_eq!(
        retried.error.as_deref(),
        Some("meeting_transcription_failed")
    );
}

#[test]
fn recovery_preserves_the_committed_stop_boundary_before_drained_packet_end() {
    let f = Fixture::new();
    let mut writer = f.writer();
    writer.append(packet(0)).unwrap(); // Packet spans QPC100..1350.
    writer
        .finish(MeetingStatus::Recorded, Some(1000), None)
        .unwrap();
    let recovered = f.store.recover("synthetic").unwrap();
    assert_eq!(recovered.end_qpc_100ns, Some(1000));
    assert_eq!(recovered.chunks[0].end_qpc_100ns, 1350);
}

#[test]
fn processing_lease_keeps_transcript_and_raw_together_until_explicit_delete() {
    let f = Fixture::new();
    let mut writer = f.writer();
    writer.append(packet(0)).unwrap();
    let original = writer
        .finish(MeetingStatus::Recorded, Some(1350), None)
        .unwrap();
    assert!(f.store.read_transcript("synthetic").unwrap().is_none());
    let mut processing = f.store.edit("synthetic").unwrap();
    assert_eq!(f.store.delete("synthetic"), Err("meeting_active"));
    processing
        .checkpoint(MeetingStatus::Processing, Some(1350), None)
        .unwrap();
    let document = br#"{"id":"synthetic","complete":true,"segments":[],"warnings":[]}"#;
    processing.write_transcript(document).unwrap();
    assert!(f
        .store
        .prune(original.created_at_unix_secs + 8 * 86400)
        .unwrap()
        .is_empty());
    processing
        .finish(MeetingStatus::Complete, Some(1350), None)
        .unwrap();
    assert_eq!(
        f.store.read_transcript("synthetic").unwrap().unwrap(),
        document
    );
    f.store.delete("synthetic").unwrap();
    assert!(!f.root.join("synthetic").exists());
}

#[test]
fn processing_edits_refuse_active_or_unrecovered_audio() {
    let f = Fixture::new();
    let mut writer = f.writer();
    writer.append(packet(0)).unwrap();
    assert!(matches!(f.store.edit("synthetic"), Err("meeting_active")));
    drop(writer);
    assert!(matches!(
        f.store.edit("synthetic"),
        Err("meeting_needs_recovery")
    ));
    f.store.recover("synthetic").unwrap();
    assert!(f.store.edit("synthetic").is_ok());
}

#[test]
fn invalid_transcript_cannot_replace_the_previous_document() {
    let f = Fixture::new();
    f.writer()
        .finish(MeetingStatus::Recorded, Some(100), None)
        .unwrap();
    let mut editor = f.store.edit("synthetic").unwrap();
    let original = br#"{"id":"synthetic","complete":false,"segments":[]}"#;
    editor.write_transcript(original).unwrap();
    assert_eq!(
        editor.write_transcript(br#"{"id":"another"}"#),
        Err("meeting_transcript")
    );
    assert_eq!(
        editor.write_transcript(b"not JSON"),
        Err("meeting_transcript")
    );
    assert_eq!(
        f.store.read_transcript("synthetic").unwrap().unwrap(),
        original
    );
}
struct Fixture {
    root: PathBuf,
    store: MeetingStore,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::current_dir()
            .unwrap()
            .join("outputs")
            .join(format!(
                "qingshu-store-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        let store = MeetingStore::new(&root).unwrap();
        Self { root, store }
    }
    fn writer(&self) -> MeetingWriter {
        self.store
            .create("synthetic", "Synthetic", 100, vec![])
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn packet(sequence: u64) -> CapturePacket {
    CapturePacket {
        source: Source::Microphone,
        epoch: 0,
        sequence,
        device_frame_position: sequence * 2,
        qpc_100ns: 100 + sequence * 1250,
        frames: 2,
        flags: 0,
        format: AudioFormat {
            sample_rate: 16000,
            channels: 1,
            bits_per_sample: 16,
            valid_bits: 16,
            channel_mask: 4,
            encoding: AudioEncoding::Pcm,
        },
        data: vec![1, 2, 3, 4],
    }
}

#[test]
fn literal_packet_and_empty_silent_packet_roundtrip() {
    let f = Fixture::new();
    let mut w = f.writer();
    w.append(packet(0)).unwrap();
    let mut silent = packet(1);
    silent.flags = 2;
    silent.data.clear();
    w.append(silent).unwrap();
    let m = w.finish(MeetingStatus::Recorded, Some(2600), None).unwrap();
    assert_eq!(m.chunks[0].frames, 4);
    assert_eq!(m.chunks[0].committed_bytes, 92);
    let raw = fs::read(f.root.join("synthetic").join(&m.chunks[0].file)).unwrap();
    assert_eq!(&raw[..8], b"QSMEET01");
    assert_eq!(&raw[8..12], &[0, 0, 0, 0]);
    assert_eq!(&raw[36..40], &[2, 0, 0, 0]);
    assert_eq!(&raw[44..48], &[4, 0, 0, 0]);
    let mut reader = f.store.open_reader("synthetic", &m.chunks[0].file).unwrap();
    assert_eq!(
        reader.next_packet().unwrap().unwrap().data,
        vec![1, 2, 3, 4]
    );
    let s = reader.next_packet().unwrap().unwrap();
    assert_eq!(s.frames, 2);
    assert!(s.data.is_empty());
    assert!(reader.next_packet().unwrap().is_none());
}

#[test]
fn interleaved_sources_epochs_and_formats_get_independent_chunks() {
    let f = Fixture::new();
    let mut w = f.writer();
    w.append(packet(0)).unwrap();
    let mut remote = packet(0);
    remote.source = Source::Remote;
    w.append(remote).unwrap();
    let mut epoch = packet(1);
    epoch.epoch = 1;
    w.append(epoch).unwrap();
    let mut format = packet(2);
    format.epoch = 1;
    format.format.sample_rate = 48000;
    w.append(format).unwrap();
    let m = w.finish(MeetingStatus::Recorded, None, None).unwrap();
    assert_eq!(m.chunks.len(), 4);
    assert!(m.chunks.iter().all(|c| c.finalized));
    assert_eq!(m.chunks.iter().map(|c| c.frames).sum::<u64>(), 8);
}

#[test]
fn duration_rollover_is_bounded_even_for_silent_packets() {
    let f = Fixture::new();
    let mut w = f.writer();
    for i in 0..12 {
        let mut p = packet(i);
        p.frames = 16000;
        p.flags = 2;
        p.data.clear();
        p.qpc_100ns = 100 + i * 10_000_000;
        w.append(p).unwrap();
    }
    let m = w.finish(MeetingStatus::Recorded, None, None).unwrap();
    assert_eq!(m.chunks.len(), 3);
    assert_eq!(m.chunks.iter().map(|c| c.frames).sum::<u64>(), 192000);
    assert!(m.chunks.iter().all(|c| c.frames <= 80000));
}

#[test]
fn crash_recovery_keeps_only_valid_prefix_and_marks_interrupted() {
    let f = Fixture::new();
    let mut w = f.writer();
    w.append(packet(0)).unwrap();
    w.checkpoint(MeetingStatus::Recording, None, None).unwrap();
    w.append(packet(1)).unwrap();
    drop(w);
    let m = f.store.load("synthetic").unwrap();
    let path = f.root.join("synthetic").join(&m.chunks[0].file);
    OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(&[0, 1, 2])
        .unwrap();
    let recovered = f.store.recover("synthetic").unwrap();
    assert_eq!(recovered.status, MeetingStatus::Interrupted);
    assert_eq!(recovered.error.as_deref(), Some("meeting_recovered"));
    assert_eq!(recovered.chunks[0].packets, 2);
    assert_eq!(recovered.chunks[0].frames, 4);
    assert_eq!(fs::metadata(path).unwrap().len(), 96);
}

#[test]
fn checkpoint_failure_keeps_previous_manifest() {
    let f = Fixture::new();
    let mut w = f.writer();
    w.append(packet(0)).unwrap();
    let path = f.root.join("synthetic/manifest.json");
    let before = fs::read(&path).unwrap();
    // An open Windows manifest denies delete/rename; no production fault hook is required.
    #[cfg(windows)]
    let held = {
        use std::os::windows::fs::OpenOptionsExt;
        OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&path)
            .unwrap()
    };
    #[cfg(windows)]
    {
        assert!(w.checkpoint(MeetingStatus::Paused, None, None).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        drop(held);
    }
    #[cfg(not(windows))]
    {
        let _ = before;
    }
}

#[test]
fn malicious_ids_manifest_paths_and_active_deletion_are_rejected() {
    let f = Fixture::new();
    for id in [
        "",
        "..",
        "../escape",
        "a/b",
        "C:evil",
        "á",
        "CON",
        "NUL",
        "com1",
    ] {
        assert!(f.store.create(id, "", 0, vec![]).is_err(), "{id}");
    }
    let w = f.writer();
    assert!(f.store.delete("synthetic").is_err());
    assert!(f.store.recover("synthetic").is_err());
    drop(w);
    let path = f.root.join("synthetic/manifest.json");
    let mut m = f.store.load("synthetic").unwrap();
    m.chunks.push(ChunkMeta {
        file: "../escape".into(),
        source: Source::Microphone,
        epoch: 0,
        format: packet(0).format,
        start_qpc_100ns: 0,
        end_qpc_100ns: 0,
        packets: 0,
        frames: 0,
        committed_bytes: 8,
        finalized: false,
    });
    fs::write(&path, serde_json::to_vec(&m).unwrap()).unwrap();
    assert!(f.store.load("synthetic").is_err());
    assert!(f.store.delete("synthetic").is_err());
}

#[test]
fn seven_day_prune_preserves_live_writer_and_recent_sessions() {
    let f = Fixture::new();
    let w = f.writer();
    let mut m = f.store.load("synthetic").unwrap();
    m.created_at_unix_secs = 1;
    fs::write(
        f.root.join("synthetic/manifest.json"),
        serde_json::to_vec(&m).unwrap(),
    )
    .unwrap();
    assert!(f.store.prune(604801).unwrap().is_empty());
    drop(w);
    assert_eq!(f.store.prune(604801).unwrap(), vec!["synthetic"]);
    let recent = f
        .store
        .create("recent", "", 0, vec![])
        .unwrap()
        .finish(MeetingStatus::Recorded, None, None)
        .unwrap();
    assert!(f
        .store
        .prune(recent.created_at_unix_secs + 604799)
        .unwrap()
        .is_empty());
}

#[test]
fn invalid_packet_is_rejected_before_writing() {
    let f = Fixture::new();
    let mut w = f.writer();
    let mut bad = packet(0);
    bad.data.pop();
    assert!(w.append(bad).is_err());
    let mut bad = packet(0);
    bad.format.bits_per_sample = 64;
    assert!(w.append(bad).is_err());
    let mut bad = packet(0);
    bad.frames = u32::MAX;
    bad.flags = 2;
    bad.data.clear();
    assert!(w.append(bad).is_err());
    assert!(w
        .finish(MeetingStatus::Recorded, None, None)
        .unwrap()
        .chunks
        .is_empty());
}

#[test]
fn committed_reader_rejects_extra_or_truncated_bytes() {
    let f = Fixture::new();
    let mut w = f.writer();
    w.append(packet(0)).unwrap();
    let m = w.finish(MeetingStatus::Recorded, None, None).unwrap();
    let path = f.root.join("synthetic").join(&m.chunks[0].file);
    OpenOptions::new()
        .write(true)
        .open(path)
        .unwrap()
        .set_len(49)
        .unwrap();
    assert!(f.store.open_reader("synthetic", &m.chunks[0].file).is_err());
}

#[test]
fn packetless_remote_heartbeat_is_persisted_per_epoch() {
    let f = Fixture::new();
    let mut w = f.writer();
    w.heartbeat(Source::Remote, 0, 100).unwrap();
    w.heartbeat(Source::Remote, 0, 1_000_100).unwrap();
    w.heartbeat(Source::Remote, 1, 2_000_100).unwrap();
    let m = w
        .finish(MeetingStatus::Recorded, Some(2_000_100), None)
        .unwrap();
    assert!(m.chunks.is_empty());
    assert_eq!(m.source_health.len(), 2);
    assert_eq!(m.source_health[0].first_heartbeat_qpc_100ns, 100);
    assert_eq!(m.source_health[0].last_heartbeat_qpc_100ns, 1_000_100);
    assert_eq!(f.store.load("synthetic").unwrap().source_health[1].epoch, 1);
}

#[test]
fn new_chunk_metadata_survives_before_first_packet_checkpoint() {
    let f = Fixture::new();
    let mut w = f.writer();
    w.append(packet(0)).unwrap();
    let committed = f.store.load("synthetic").unwrap();
    assert_eq!(committed.chunks.len(), 1);
    assert_eq!(committed.chunks[0].packets, 0);
    assert_eq!(committed.chunks[0].committed_bytes, 8);
    drop(w);
    let recovered = f.store.recover("synthetic").unwrap();
    assert_eq!(recovered.chunks[0].packets, 1);
    assert_eq!(recovered.chunks[0].frames, 2);
}

#[test]
fn malicious_tail_payload_length_does_not_allocate_or_lose_valid_prefix() {
    let f = Fixture::new();
    let mut w = f.writer();
    w.append(packet(0)).unwrap();
    drop(w);
    let m = f.store.load("synthetic").unwrap();
    let mut file = OpenOptions::new()
        .append(true)
        .open(f.root.join("synthetic").join(&m.chunks[0].file))
        .unwrap();
    let mut bad = [0; 40];
    bad[28..32].copy_from_slice(&2u32.to_le_bytes());
    bad[36..40].copy_from_slice(&u32::MAX.to_le_bytes());
    file.write_all(&bad).unwrap();
    drop(file);
    let m = f.store.recover("synthetic").unwrap();
    assert_eq!(m.chunks[0].packets, 1);
    assert_eq!(m.chunks[0].committed_bytes, 52);
}

#[test]
fn file_size_rollover_preserves_exact_frames_and_never_exceeds_one_packet_overhead() {
    let f = Fixture::new();
    let mut w = f.writer();
    for i in 0..5 {
        let mut p = packet(i);
        p.format.sample_rate = 384000;
        p.format.channels = 32;
        p.frames = 16384;
        p.data = vec![0; 1_048_576];
        w.append(p).unwrap();
    }
    let m = w.finish(MeetingStatus::Recorded, None, None).unwrap();
    assert_eq!(m.chunks.len(), 2);
    assert_eq!(m.chunks[0].packets, 3);
    assert_eq!(m.chunks[1].packets, 2);
    assert_eq!(m.chunks.iter().map(|c| c.frames).sum::<u64>(), 81920);
    assert!(m.chunks.iter().all(|c| c.committed_bytes <= 4_194_304));
}

#[test]
#[ignore = "native link creation permission; run check-meeting-store.py --links"]
fn hardlinked_audio_file_is_never_recovered_or_deleted() {
    let f = Fixture::new();
    let mut w = f.writer();
    w.append(packet(0)).unwrap();
    let m = w.finish(MeetingStatus::Recorded, None, None).unwrap();
    let audio = f.root.join("synthetic").join(&m.chunks[0].file);
    let alias = f.root.join("outside-alias.bin");
    fs::hard_link(&audio, &alias).unwrap();
    let before = fs::read(&alias).unwrap();
    assert!(f.store.recover("synthetic").is_err());
    assert!(f.store.delete("synthetic").is_err());
    assert_eq!(fs::read(alias).unwrap(), before);
}

#[test]
#[ignore = "native link creation permission; run check-meeting-store.py --links"]
fn symlink_or_junction_session_refuses_read_recovery_and_deletion() {
    let f = Fixture::new();
    let outside = f.root.join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("sentinel"), b"safe").unwrap();
    let link = f.root.join("linked");
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
            "junction fixture creation must succeed"
        );
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, &link).unwrap();
    assert!(f.store.load("linked").is_err());
    assert!(f.store.recover("linked").is_err());
    assert!(f.store.delete("linked").is_err());
    assert_eq!(fs::read(outside.join("sentinel")).unwrap(), b"safe");
}

#[test]
fn production_queue_absorbs_ten_seconds_then_full_remains_explicit_and_lossless() {
    use std::sync::mpsc::TrySendError;
    let (sender, receiver) = capture_event_queue();
    for sequence in 0..2048 {
        sender
            .try_send(CaptureEvent::Packet(packet(sequence)))
            .unwrap();
    }
    assert!(matches!(
        sender.try_send(CaptureEvent::Packet(packet(2048))),
        Err(TrySendError::Full(_))
    ));
    drop(sender);
    let mut count = 0;
    while let Ok(event) = receiver.recv() {
        if let CaptureEvent::Packet(p) = event {
            assert_eq!(p.sequence, count);
            count += 1;
        }
    }
    assert_eq!(count, 2048);
}

struct CountWrite<W> {
    inner: W,
    calls: usize,
    bytes: usize,
}
impl<W: Write> Write for CountWrite<W> {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.calls += 1;
        let n = self.inner.write(data)?;
        self.bytes += n;
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

#[test]
fn manifest_serialization_batches_system_writes_at_two_hour_metadata_scale() {
    let f = Fixture::new();
    let w = f.writer();
    let mut m = w.manifest().clone();
    drop(w);
    let mut observations = vec![];
    for count in [0, 100, 1000, 2880] {
        m.chunks = (0..count)
            .map(|i| ChunkMeta {
                file: format!("chunk-{i:08}.bin"),
                source: if i % 2 == 0 {
                    Source::Microphone
                } else {
                    Source::Remote
                },
                epoch: 0,
                format: packet(0).format,
                start_qpc_100ns: i as u64 * 50_000_000,
                end_qpc_100ns: (i as u64 + 1) * 50_000_000,
                packets: 500,
                frames: 240000,
                committed_bytes: 1_000_008,
                finalized: true,
            })
            .collect();
        let file = File::create(f.root.join("manifest-benchmark.json")).unwrap();
        let mut meter = CountWrite {
            inner: file,
            calls: 0,
            bytes: 0,
        };
        let began = Instant::now();
        serialize_manifest_to(&mut meter, &m).unwrap();
        meter.flush().unwrap();
        meter.inner.sync_all().unwrap();
        println!(
            "{}",
            serde_json::json!({"benchmark":"production_manifest_serialization","chunks":count,"bytes":meter.bytes,"system_write_calls":meter.calls,"elapsed_us":began.elapsed().as_micros()})
        );
        observations.push((meter.bytes, meter.calls));
    }
    let (bytes, calls) = observations.last().copied().unwrap();
    assert!(calls <= bytes.div_ceil(64*1024)+1,"manifest JSON tokens must be batched before operating-system writes: bytes={bytes} calls={calls}");
}

#[test]
fn checkpoint_if_due_deduplicates_automatic_and_external_ticks() {
    let f = Fixture::new();
    let mut w = f.writer();
    w.append(packet(0)).unwrap();
    let before = w.stats().checkpoints;
    w.last_checkpoint = Instant::now() - Duration::from_secs(2);
    assert!(w.checkpoint_if_due().unwrap());
    assert_eq!(w.stats().checkpoints, before + 1);
    assert!(!w.checkpoint_if_due().unwrap());
    assert_eq!(w.stats().checkpoints, before + 1);
}

struct ReadFailsAfter {
    inner: File,
    left: usize,
}
impl Read for ReadFailsAfter {
    fn read(&mut self, data: &mut [u8]) -> std::io::Result<usize> {
        if self.left == 0 {
            return Err(std::io::Error::other("synthetic_read_io"));
        }
        let take = data.len().min(self.left);
        let n = self.inner.read(&mut data[..take])?;
        self.left -= n;
        Ok(n)
    }
}
#[test]
fn recovery_read_io_after_valid_packet_preserves_every_original_byte() {
    let f = Fixture::new();
    let mut w = f.writer();
    w.append(packet(0)).unwrap();
    w.append(packet(1)).unwrap();
    drop(w);
    let before_manifest = fs::read(f.root.join("synthetic/manifest.json")).unwrap();
    let m = f.store.load("synthetic").unwrap();
    let path = f.root.join("synthetic").join(&m.chunks[0].file);
    let before = fs::read(&path).unwrap();
    let result = f.store.recover_with_scanner("synthetic", |path, meta| {
        scan_chunk(
            ReadFailsAfter {
                inner: File::open(path).unwrap(),
                left: 52,
            },
            meta.clone(),
            regular(path)?.len(),
        )
    });
    assert_eq!(result.err(), Some("meeting_io"));
    assert_eq!(fs::read(path).unwrap(), before);
    assert_eq!(
        fs::read(f.root.join("synthetic/manifest.json")).unwrap(),
        before_manifest
    );
}
#[test]
fn later_chunk_read_io_does_not_truncate_an_earlier_bad_tail() {
    let f = Fixture::new();
    let mut w = f.writer();
    w.append(packet(0)).unwrap();
    let mut remote = packet(0);
    remote.source = Source::Remote;
    w.append(remote).unwrap();
    drop(w);
    let m = f.store.load("synthetic").unwrap();
    let first = f.root.join("synthetic").join(&m.chunks[0].file);
    let second = f.root.join("synthetic").join(&m.chunks[1].file);
    OpenOptions::new()
        .append(true)
        .open(&first)
        .unwrap()
        .write_all(&[9, 8, 7])
        .unwrap();
    let before_first = fs::read(&first).unwrap();
    let before_second = fs::read(&second).unwrap();
    let before_manifest = fs::read(f.root.join("synthetic/manifest.json")).unwrap();
    let result = f.store.recover_with_scanner("synthetic", |path, meta| {
        scan_chunk(
            ReadFailsAfter {
                inner: File::open(path).unwrap(),
                left: if meta.source == Source::Remote {
                    8
                } else {
                    usize::MAX
                },
            },
            meta.clone(),
            regular(path)?.len(),
        )
    });
    assert_eq!(result.err(), Some("meeting_io"));
    assert_eq!(fs::read(first).unwrap(), before_first);
    assert_eq!(fs::read(second).unwrap(), before_second);
    assert_eq!(
        fs::read(f.root.join("synthetic/manifest.json")).unwrap(),
        before_manifest
    );
}

#[test]
#[ignore = "controlled throughput benchmark; run this named test explicitly"]
fn two_hour_manifest_scale_preserves_native_packet_writer_throughput() {
    for count in [0, 100, 1000, 2880] {
        let f = Fixture::new();
        let mut w = f.writer();
        // Metadata-only scale fixture, not a claim that these audio files were recorded.
        w.manifest.chunks = (0..count)
            .map(|i| ChunkMeta {
                file: format!("chunk-{i:08}.bin"),
                source: if i % 2 == 0 {
                    Source::Microphone
                } else {
                    Source::Remote
                },
                epoch: 0,
                format: AudioFormat {
                    sample_rate: 48000,
                    channels: 1,
                    bits_per_sample: 32,
                    valid_bits: 32,
                    channel_mask: 4,
                    encoding: AudioEncoding::Float,
                },
                start_qpc_100ns: i as u64 * 50_000_000,
                end_qpc_100ns: (i as u64 + 1) * 50_000_000,
                packets: 500,
                frames: 240000,
                committed_bytes: 1_000_008,
                finalized: true,
            })
            .collect();
        let began = Instant::now();
        for sequence in 0..500 {
            for source in [Source::Microphone, Source::Remote] {
                let channels = if source == Source::Microphone { 1 } else { 2 };
                let p = CapturePacket {
                    source,
                    epoch: 0,
                    sequence,
                    device_frame_position: sequence * 480,
                    qpc_100ns: 10_000_000 + sequence * 100_000,
                    frames: 480,
                    flags: 0,
                    format: AudioFormat {
                        sample_rate: 48000,
                        channels,
                        bits_per_sample: 32,
                        valid_bits: 32,
                        channel_mask: if channels == 1 { 4 } else { 3 },
                        encoding: AudioEncoding::Float,
                    },
                    data: vec![0; 480 * channels as usize * 4],
                };
                w.append(p).unwrap();
            }
            // Force the same five periodic checkpoints a real five-second source would need.
            if sequence % 100 == 99 {
                w.last_checkpoint = Instant::now() - Duration::from_secs(2);
                assert!(w.checkpoint_if_due().unwrap());
            }
        }
        let elapsed = began.elapsed();
        println!(
            "{}",
            serde_json::json!({"benchmark":"production_native_append_with_metadata_scale","preexisting_metadata_chunks":count,"native_packets":1000,"native_sample_duration_seconds":5,"actual_elapsed_seconds":elapsed.as_secs_f64(),"packets_per_second":1000.0/elapsed.as_secs_f64(),"max_checkpoint_us":w.stats().max_checkpoint_us})
        );
        assert!(elapsed<Duration::from_secs(5),"1000 native packets and checkpoints must fit within their five-second source duration at {count} metadata chunks");
    }
}
