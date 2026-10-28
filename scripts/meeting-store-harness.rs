// Synthetic sources only. Uses the production CaptureEvent bounded queue and packet writer.
use crate::queue::capture_event_queue;
use crate::store::{MeetingStore, StoreResult};
use crate::types::*;
use std::{
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        mpsc::TrySendError,
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

#[cfg(windows)]
fn memory_bytes() -> (usize, usize) {
    #[repr(C)]
    struct Counters {
        cb: u32,
        faults: u32,
        peak_working: usize,
        working: usize,
        peak_paged: usize,
        paged: usize,
        peak_nonpaged: usize,
        nonpaged: usize,
        pagefile: usize,
        peak_pagefile: usize,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut std::ffi::c_void;
    }
    #[link(name = "psapi")]
    unsafe extern "system" {
        fn GetProcessMemoryInfo(
            process: *mut std::ffi::c_void,
            counters: *mut Counters,
            size: u32,
        ) -> i32;
    }
    let mut c: Counters = unsafe { std::mem::zeroed() };
    c.cb = std::mem::size_of::<Counters>() as u32;
    if unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) } == 0 {
        (0, 0)
    } else {
        (c.working, c.peak_working)
    }
}
#[cfg(not(windows))]
fn memory_bytes() -> (usize, usize) {
    (0, 0)
}

fn source_packet(source: Source, sequence: u64, microphone_channels: u16) -> CapturePacket {
    let channels = if source == Source::Microphone {
        microphone_channels
    } else {
        2
    };
    let silent = sequence % 1000 >= 950;
    let mut data = Vec::with_capacity(480 * channels as usize * 4);
    if !silent {
        for frame in 0..480 {
            let phase = ((sequence * 480 + frame) % 127) as f32;
            let sample = (phase - 63.0) / 640.0;
            for _ in 0..channels {
                data.extend_from_slice(&sample.to_le_bytes());
            }
        }
    }
    // Remote clock is deliberately +75ppm. No audio devices or environment are touched.
    let qpc = 10_000_000
        + sequence * 100_000
        + if source == Source::Remote {
            sequence * 75 / 10
        } else {
            0
        };
    CapturePacket {
        source,
        epoch: 0,
        sequence,
        device_frame_position: sequence * 480,
        qpc_100ns: qpc,
        frames: 480,
        flags: if silent { 2 } else { 0 },
        format: AudioFormat {
            sample_rate: 48000,
            channels,
            bits_per_sample: 32,
            valid_bits: 32,
            channel_mask: if channels == 1 { 4 } else { 3 },
            encoding: AudioEncoding::Float,
        },
        data,
    }
}

fn run() -> StoreResult<()> {
    let args: Vec<String> = std::env::args().collect();
    let value = |name: &str| args.windows(2).find(|a| a[0] == name).map(|a| a[1].clone());
    let root = value("--root").ok_or("harness_root_required")?;
    let id = value("--id").unwrap_or_else(|| "synthetic-soak".into());
    let seconds: u64 = value("--seconds")
        .unwrap_or_else(|| "10".into())
        .parse()
        .map_err(|_| "harness_seconds")?;
    if seconds == 0 || seconds > 86400 {
        return Err("harness_seconds");
    }
    let microphone_channels: u16 = value("--microphone-channels")
        .unwrap_or_else(|| "1".into())
        .parse()
        .map_err(|_| "harness_channels")?;
    if !matches!(microphone_channels, 1 | 2) {
        return Err("harness_channels");
    }
    let accelerated = args.iter().any(|a| a == "--accelerated");
    let stall_ms: u64 = value("--stall-checkpoint-ms")
        .unwrap_or_else(|| "0".into())
        .parse()
        .map_err(|_| "harness_stall")?;
    let stall_at_ms: u64 = value("--stall-at-ms")
        .unwrap_or_else(|| "1000".into())
        .parse()
        .map_err(|_| "harness_stall")?;
    let store = MeetingStore::new(root)?;
    let mut writer = store.create(&id, "Synthetic controlled soak", 10_000_000, vec![])?;
    let (sender, receiver) = capture_event_queue();
    let queued = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let producer_queued = queued.clone();
    let producer_peak = peak.clone();
    let max_producer_lateness_us = Arc::new(AtomicU64::new(0));
    let producer_lateness = max_producer_lateness_us.clone();
    let started = Instant::now();
    println!(
        "{}",
        serde_json::json!({"event":"start", "mode":if accelerated {"accelerated_sample_duration"} else {"actual_elapsed"}, "planned_seconds":seconds, "queue_capacity":MEETING_CAPTURE_QUEUE_CAPACITY,"queue_payload_budget_bytes":MEETING_CAPTURE_QUEUE_MAX_BYTES,"sample_rate":48000,"packet_frames":480,"sources":2,"microphone_channels":microphone_channels,"remote_channels":2,"controlled_checkpoint_stall_ms":stall_ms})
    );
    let producer = thread::spawn(move || -> StoreResult<()> {
        let send = |event| -> StoreResult<()> {
            // Count before publication so a fast consumer cannot underflow the counter.
            let depth = producer_queued.fetch_add(1, Ordering::SeqCst) + 1;
            match sender.try_send(event) {
                Ok(()) => {
                    producer_peak
                        .fetch_max(depth.min(MEETING_CAPTURE_QUEUE_CAPACITY), Ordering::Relaxed);
                    Ok(())
                }
                Err(TrySendError::Full(_)) => {
                    producer_queued.fetch_sub(1, Ordering::SeqCst);
                    Err("harness_queue_full")
                }
                Err(TrySendError::Disconnected(_)) => {
                    producer_queued.fetch_sub(1, Ordering::SeqCst);
                    Err("harness_disconnected")
                }
            }
        };
        for sequence in 0..seconds * 100 {
            if !accelerated {
                let due = Duration::from_millis(sequence * 10);
                if let Some(wait) = due.checked_sub(started.elapsed()) {
                    thread::sleep(wait);
                }
                producer_lateness.fetch_max(
                    started
                        .elapsed()
                        .saturating_sub(due)
                        .as_micros()
                        .min(u64::MAX as u128) as u64,
                    Ordering::Relaxed,
                );
            }
            for source in [Source::Microphone, Source::Remote] {
                // Accelerated input still exercises a bounded queue without asking it to hold hours.
                if accelerated {
                    while producer_queued.load(Ordering::SeqCst)
                        >= MEETING_CAPTURE_QUEUE_CAPACITY - 4
                    {
                        thread::sleep(Duration::from_millis(1));
                    }
                }
                send(CaptureEvent::Packet(source_packet(
                    source,
                    sequence,
                    microphone_channels,
                )))?;
            }
            if sequence % 100 == 0 {
                for source in [Source::Microphone, Source::Remote] {
                    if accelerated {
                        while producer_queued.load(Ordering::SeqCst)
                            >= MEETING_CAPTURE_QUEUE_CAPACITY - 4
                        {
                            thread::sleep(Duration::from_millis(1));
                        }
                    }
                    send(CaptureEvent::Heartbeat {
                        source,
                        qpc_100ns: 10_000_000 + sequence * 100_000,
                    })?;
                }
            }
        }
        if !accelerated {
            if let Some(wait) = Duration::from_secs(seconds).checked_sub(started.elapsed()) {
                thread::sleep(wait);
            }
        }
        Ok(())
    });
    let mut frames = [0u64; 2];
    let mut packets = 0u64;
    let mut reported = 0;
    let mut checkpoint = Instant::now();
    let mut write_error = None;
    let mut writer_max_op_us = 0u64;
    let mut checkpoint_stalled = false;
    while let Ok(event) = receiver.recv() {
        queued.fetch_sub(1, Ordering::SeqCst);
        let operation_began = Instant::now();
        let result = match event {
            CaptureEvent::Packet(packet) => {
                let i = if packet.source == Source::Microphone {
                    0
                } else {
                    1
                };
                frames[i] += packet.frames as u64;
                packets += 1;
                writer.append(packet)
            }
            CaptureEvent::Heartbeat { source, qpc_100ns } => writer.heartbeat(source, 0, qpc_100ns),
            CaptureEvent::Error { .. } => Err("harness_source_error"),
        };
        writer_max_op_us = writer_max_op_us
            .max(operation_began.elapsed().as_micros().min(u64::MAX as u128) as u64);
        if let Err(error) = result {
            write_error = Some(error);
            break;
        }
        if checkpoint.elapsed() >= Duration::from_secs(1) {
            if !checkpoint_stalled
                && stall_ms != 0
                && started.elapsed().as_millis() >= stall_at_ms as u128
            {
                // Controlled consumer stall, outside production code, before a real checkpoint.
                checkpoint_stalled = true;
                println!(
                    "{}",
                    serde_json::json!({"event":"controlled_checkpoint_stall", "elapsed_seconds":started.elapsed().as_secs_f64(), "duration_ms":stall_ms})
                );
                thread::sleep(Duration::from_millis(stall_ms));
            }
            let operation_began = Instant::now();
            writer.checkpoint_if_due()?;
            writer_max_op_us = writer_max_op_us
                .max(operation_began.elapsed().as_micros().min(u64::MAX as u128) as u64);
            checkpoint = Instant::now();
        }
        let elapsed = started.elapsed().as_secs();
        if elapsed >= reported + 60 {
            reported = elapsed;
            let (rss, peak_rss) = memory_bytes();
            println!(
                "{}",
                serde_json::json!({"event":"progress","elapsed_seconds":elapsed,"frames":frames,"packets":packets,"queue_peak":peak.load(Ordering::Relaxed),"working_set_bytes":rss,"peak_working_set_bytes":peak_rss,"chunks":writer.manifest().chunks.len(),"writer_max_op_us":writer_max_op_us,"max_checkpoint_us":writer.stats().max_checkpoint_us,"max_producer_lateness_us":max_producer_lateness_us.load(Ordering::Relaxed)})
            );
        }
    }
    drop(receiver);
    let source_result = producer.join().map_err(|_| "harness_producer_panic")?;
    let stats = writer.stats();
    println!(
        "{}",
        serde_json::json!({"event":"diagnostics", "elapsed_seconds":started.elapsed().as_secs_f64(), "frames":frames,"packets":packets,"queue_peak":peak.load(Ordering::Relaxed),"writer_max_op_us":writer_max_op_us,"max_checkpoint_us":stats.max_checkpoint_us,"checkpoints":stats.checkpoints,"max_producer_lateness_us":max_producer_lateness_us.load(Ordering::Relaxed)})
    );
    if let Some(error) = write_error {
        let _ = writer.finish(MeetingStatus::Interrupted, None, Some(error.into()));
        return Err(error);
    }
    if let Err(error) = source_result {
        let _ = writer.finish(MeetingStatus::Interrupted, None, Some(error.into()));
        return Err(error);
    }
    if frames != [seconds * 48000, seconds * 48000] {
        return Err("harness_frames");
    }
    let manifest = writer.finish(
        MeetingStatus::Recorded,
        Some(10_000_000 + seconds * 10_000_000),
        None,
    )?;
    // Recovery itself streams every native packet and checks complete accounting.
    let recovered = store.recover(&id)?;
    let recovered_frames: u64 = recovered.chunks.iter().map(|c| c.frames).sum();
    let recovered_packets: u64 = recovered.chunks.iter().map(|c| c.packets).sum();
    if recovered_frames != seconds * 96000 || recovered_packets != seconds * 200 {
        return Err("harness_recovery_accounting");
    }
    // Recreate one expected synthetic packet at a time. This checks exact bytes as well as timing/flags.
    let mut verified_sequences = [0u64; 2];
    let verified_began = Instant::now();
    for meta in &recovered.chunks {
        let source = meta.source;
        let source_index = if source == Source::Microphone { 0 } else { 1 };
        let mut reader = store.open_reader_for_chunk(&id, meta)?;
        while let Some(packet) = reader.next_packet()? {
            let expected = source_packet(
                source,
                verified_sequences[source_index],
                microphone_channels,
            );
            if packet.source != expected.source
                || packet.epoch != expected.epoch
                || packet.sequence != expected.sequence
                || packet.device_frame_position != expected.device_frame_position
                || packet.qpc_100ns != expected.qpc_100ns
                || packet.frames != expected.frames
                || packet.flags != expected.flags
                || packet.format != expected.format
                || packet.data != expected.data
            {
                return Err("harness_exact_packet_roundtrip");
            }
            verified_sequences[source_index] += 1;
        }
    }
    if verified_sequences != [seconds * 100, seconds * 100] {
        return Err("harness_exact_sequence_accounting");
    }
    let elapsed = started.elapsed().as_secs_f64();
    if !accelerated && elapsed < seconds as f64 {
        return Err("harness_elapsed_short");
    }
    let (rss, peak_rss) = memory_bytes();
    println!(
        "{}",
        serde_json::json!({"event":"complete","mode":if accelerated {"accelerated_sample_duration"} else {"actual_elapsed"},"planned_seconds":seconds,"actual_elapsed_seconds":elapsed,"frames":frames,"packets":packets,"chunks":manifest.chunks.len(),"bytes":manifest.chunks.iter().map(|c|c.committed_bytes).sum::<u64>(),"queue_peak":peak.load(Ordering::Relaxed),"working_set_bytes":rss,"peak_working_set_bytes":peak_rss,"recovery_frames":recovered_frames,"recovery_packets":recovered_packets,"recovery_status":"interrupted","accounting":"pass","exact_packet_roundtrip":"pass","verified_sequences":verified_sequences,"verification_seconds":verified_began.elapsed().as_secs_f64(),"writer_max_op_us":writer_max_op_us,"max_checkpoint_us":stats.max_checkpoint_us,"max_producer_lateness_us":max_producer_lateness_us.load(Ordering::Relaxed)})
    );
    Ok(())
}
fn main() {
    if let Err(code) = run() {
        eprintln!("{{\"event\":\"failure\",\"code\":\"{code}\"}}");
        std::process::exit(1);
    }
}
