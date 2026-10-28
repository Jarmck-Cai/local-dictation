//! Standalone native capture probe. No audio starts without explicit `capture`.
//!
//! Build with rustc and the existing application's serde/specta/windows rlibs.
//! `list` enumerates endpoints only. `capture <mic-id> <render-id> <seconds>`
//! requires both exact IDs and 1..=30 seconds (including decimal `2.2`), keeps statistics only and drops
//! every packet immediately. Optional `--timings <new-path>` saves only native
//! clocks, flags and format metadata for synthetic-silence alignment replay.
//! It never saves samples, waveforms, levels or device identifiers, plays audio
//! or transcribes. Without `--timings`, it writes no file.
#[allow(dead_code, unused_imports)]
#[path = "../desktop/src-tauri/src/meetings/capture.rs"]
mod capture;
#[allow(dead_code)]
#[path = "../desktop/src-tauri/src/meetings/queue.rs"]
mod queue;
#[allow(dead_code)]
#[path = "../desktop/src-tauri/src/meetings/types.rs"]
mod types;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::Arc;
use std::time::{Duration, Instant};
use types::{CaptureConfig, CaptureEvent, CapturePacket, Source};

fn packet_timing(packet: &CapturePacket) -> serde_json::Value {
    // Explicit whitelist: never serialize CapturePacket or DeviceInfo wholesale.
    serde_json::json!({
        "source": packet.source,
        "epoch": packet.epoch,
        "sequence": packet.sequence,
        "device_frame_position": packet.device_frame_position,
        "qpc_100ns": packet.qpc_100ns,
        "frames": packet.frames,
        "flags": packet.flags,
        "format": {
            "sample_rate": packet.format.sample_rate,
            "channels": packet.format.channels,
            "bits_per_sample": packet.format.bits_per_sample,
            "valid_bits": packet.format.valid_bits,
            "channel_mask": packet.format.channel_mask,
            "encoding": packet.format.encoding,
        }
    })
}

#[derive(serde::Serialize)]
struct TimingTrace {
    schema_version: u32,
    start_qpc_100ns: u64,
    end_qpc_100ns: u64,
    packets: Vec<serde_json::Value>,
    heartbeats: Vec<serde_json::Value>,
}

impl TimingTrace {
    fn new(start_qpc_100ns: u64) -> Self {
        Self {
            schema_version: 1,
            start_qpc_100ns,
            end_qpc_100ns: 0,
            packets: vec![],
            heartbeats: vec![],
        }
    }

    fn heartbeat(&mut self, source: Source, qpc_100ns: u64) {
        self.heartbeats
            .push(serde_json::json!({"source": source, "qpc_100ns": qpc_100ns}));
    }

    fn write(&self, path: &Path) -> Result<(), String> {
        // Explicit output cannot replace an earlier file, including by a race.
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|_| "probe_timings_create")?;
        serde_json::to_writer_pretty(file, self).map_err(|_| "probe_timings_write".into())
    }
}

#[derive(Debug)]
enum Command {
    List,
    Capture {
        microphone_id: String,
        render_id: String,
        duration: Duration,
        timings: Option<PathBuf>,
    },
}

fn parse_command(args: &[String]) -> Result<Command, String> {
    if args.len() == 1 && args[0] == "list" {
        return Ok(Command::List);
    }
    if !matches!(args.len(), 4 | 6)
        || args[0] != "capture"
        || (args.len() == 6 && (args[4] != "--timings" || args[5].is_empty()))
    {
        return Err("usage: probe-meeting-capture list | capture <microphone-id> <render-id> <seconds:1..30> [--timings <new-path>]".into());
    }
    let seconds: f64 = args[3].parse().map_err(|_| "probe_duration_invalid")?;
    if !seconds.is_finite() || !(1.0..=30.0).contains(&seconds) {
        return Err("probe_duration_invalid".into());
    }
    Ok(Command::Capture {
        microphone_id: args[1].clone(),
        render_id: args[2].clone(),
        duration: Duration::from_secs_f64(seconds),
        timings: args.get(5).map(PathBuf::from),
    })
}

#[cfg(test)]
mod probe_tests {
    use super::*;

    #[test]
    fn timing_packet_preserves_clocks_and_flags_without_private_samples() {
        let packet = CapturePacket {
            source: Source::Remote,
            epoch: 7,
            sequence: 13,
            device_frame_position: 480,
            qpc_100ns: 9_100_000,
            frames: 480,
            flags: 1 | 2 | 4,
            format: types::AudioFormat {
                sample_rate: 48_000,
                channels: 2,
                bits_per_sample: 32,
                valid_bits: 32,
                channel_mask: 3,
                encoding: types::AudioEncoding::Float,
            },
            data: b"PrivateSentinel".to_vec(),
        };
        let value = packet_timing(&packet);
        assert_eq!(
            value,
            serde_json::json!({
                "source": "remote", "epoch": 7, "sequence": 13,
                "device_frame_position": 480, "qpc_100ns": 9_100_000,
                "frames": 480, "flags": 7,
                "format": {"sample_rate": 48_000, "channels": 2,
                    "bits_per_sample": 32, "valid_bits": 32, "channel_mask": 3,
                    "encoding": "float"}
            })
        );
        let encoded = serde_json::to_string(&value).unwrap();
        for private in [
            "PrivateSentinel",
            "data",
            "peak",
            "rms",
            "energy",
            "device_id",
            "name",
            "content",
        ] {
            assert!(
                !encoded.contains(private),
                "private field leaked: {private}"
            );
        }
        assert_eq!(packet.data, b"PrivateSentinel");
    }

    #[test]
    fn timing_trace_has_only_common_interval_packets_and_heartbeats() {
        let mut trace = TimingTrace::new(100);
        trace.end_qpc_100ns = 22_000_100;
        trace.heartbeat(Source::Microphone, 100);
        trace.heartbeat(Source::Remote, 22_000_100);
        assert_eq!(
            serde_json::to_value(trace).unwrap(),
            serde_json::json!({
                "schema_version": 1,
                "start_qpc_100ns": 100, "end_qpc_100ns": 22_000_100,
                "packets": [], "heartbeats": [
                    {"source": "microphone", "qpc_100ns": 100},
                    {"source": "remote", "qpc_100ns": 22_000_100}
                ]
            })
        );
    }

    #[test]
    fn device_actions_and_timing_output_require_explicit_arguments() {
        let args = |values: &[&str]| values.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(parse_command(&[]).is_err());
        assert!(matches!(parse_command(&args(&["list"])), Ok(Command::List)));
        assert!(parse_command(&args(&["list", "--timings", "unused.json"])).is_err());
        match parse_command(&args(&["capture", "mic", "render", "2.2"])).unwrap() {
            Command::Capture {
                duration, timings, ..
            } => {
                assert_eq!(duration, Duration::from_millis(2200));
                assert!(timings.is_none());
            }
            _ => panic!("expected explicit capture"),
        }
        match parse_command(&args(&[
            "capture",
            "mic",
            "render",
            "30",
            "--timings",
            "timings.json",
        ]))
        .unwrap()
        {
            Command::Capture { timings, .. } => {
                assert_eq!(timings, Some(PathBuf::from("timings.json")))
            }
            _ => panic!("expected explicit capture"),
        }
        for seconds in ["NaN", "inf", "0", "0.9", "30.1", "-1"] {
            assert!(parse_command(&args(&["capture", "mic", "render", seconds])).is_err());
        }
        assert!(parse_command(&args(&[
            "capture",
            "mic",
            "render",
            "2.2",
            "--other",
            "unused.json"
        ]))
        .is_err());
    }
}

#[derive(Default)]
struct Statistics {
    packets: u64,
    frames: u64,
    bytes: u64,
    silent_packets: u64,
    discontinuities: u64,
    timestamp_errors: u64,
    qpc_backwards: u64,
    device_backwards: u64,
    last_reliable_qpc: Option<u64>,
    last_reliable_position: Option<u64>,
    first_reliable: Option<(u64, u64)>,
    last_reliable: Option<(u64, u64)>,
    sample_rate: u32,
    peak: f64,
    energy: f64,
    samples: u64,
    non_finite: u64,
    heartbeats: u64,
    first_heartbeat: Option<u64>,
    last_heartbeat: Option<u64>,
}

impl Statistics {
    fn heartbeat(&mut self, qpc_100ns: u64) {
        self.heartbeats += 1;
        self.first_heartbeat.get_or_insert(qpc_100ns);
        self.last_heartbeat = Some(qpc_100ns);
    }

    fn covers_boundaries(&self, start: u64, end: u64) -> bool {
        self.first_heartbeat == Some(start) && self.last_heartbeat == Some(end)
    }

    fn packet(&mut self, packet: CapturePacket) {
        self.packets += 1;
        self.frames += u64::from(packet.frames);
        self.bytes += packet.data.len() as u64;
        self.sample_rate = packet.format.sample_rate;
        self.silent_packets += u64::from(packet.flags & 2 != 0);
        self.discontinuities += u64::from(packet.flags & 1 != 0);
        self.timestamp_errors += u64::from(packet.flags & 4 != 0);
        if packet.flags & 4 == 0 {
            self.qpc_backwards += u64::from(
                self.last_reliable_qpc
                    .is_some_and(|qpc| packet.qpc_100ns < qpc),
            );
            self.device_backwards += u64::from(
                self.last_reliable_position
                    .is_some_and(|position| packet.device_frame_position < position),
            );
            self.last_reliable_qpc = Some(packet.qpc_100ns);
            self.last_reliable_position = Some(packet.device_frame_position);
            let point = (packet.qpc_100ns, packet.device_frame_position);
            self.first_reliable.get_or_insert(point);
            self.last_reliable = Some(point);
        }
        // Decode for aggregate energy only; the original payload is dropped here.
        // Integer valid bits are left aligned in the original PCM container.
        if packet.flags & 2 != 0 {
            self.samples += u64::from(packet.frames) * u64::from(packet.format.channels);
            return;
        }
        let width = usize::from(packet.format.bits_per_sample / 8);
        for bytes in packet.data.chunks_exact(width) {
            let value = match (packet.format.encoding, width) {
                (types::AudioEncoding::Float, 4) => {
                    f64::from(f32::from_le_bytes(bytes.try_into().unwrap()))
                }
                (types::AudioEncoding::Pcm, 1) => (f64::from(bytes[0]) - 128.0) / 128.0,
                (types::AudioEncoding::Pcm, 2) => {
                    f64::from(i16::from_le_bytes(bytes.try_into().unwrap())) / 32768.0
                }
                (types::AudioEncoding::Pcm, 3) => {
                    let signed = i32::from_le_bytes([0, bytes[0], bytes[1], bytes[2]]);
                    f64::from(signed) / 2147483648.0
                }
                (types::AudioEncoding::Pcm, 4) => {
                    f64::from(i32::from_le_bytes(bytes.try_into().unwrap())) / 2147483648.0
                }
                _ => continue,
            };
            self.samples += 1;
            if value.is_finite() {
                self.energy += value * value;
                self.peak = self.peak.max(value.abs());
            } else {
                self.non_finite += 1;
            }
        }
    }

    fn print(&self, name: &str) {
        let rms = if self.samples == 0 {
            0.0
        } else {
            (self.energy / self.samples as f64).sqrt()
        };
        let drift_ppm = match (self.first_reliable, self.last_reliable) {
            (Some((q0, p0)), Some((q1, p1))) if q1 > q0 && p1 > p0 && self.sample_rate > 0 => {
                (((p1 - p0) as f64 * 10_000_000.0 / (q1 - q0) as f64) / f64::from(self.sample_rate)
                    - 1.0)
                    * 1_000_000.0
            }
            _ => 0.0,
        };
        println!("{name}: packets={} frames={} bytes={} silent={} discontinuities={} timestamp_errors={} qpc_backwards={} device_backwards={} rate={} observed_drift_ppm={:.3} peak={:.6} rms={:.6} non_finite={}",
            self.packets, self.frames, self.bytes, self.silent_packets, self.discontinuities,
            self.timestamp_errors, self.qpc_backwards, self.device_backwards, self.sample_rate,
            drift_ppm, self.peak, rms, self.non_finite);
        println!(
            "{name}: heartbeats={} first_heartbeat_100ns={} last_heartbeat_100ns={}",
            self.heartbeats,
            self.first_heartbeat.unwrap_or(0),
            self.last_heartbeat.unwrap_or(0)
        );
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = parse_command(&args)?;
    if matches!(command, Command::List) {
        // IDs are deliberately displayed only to this explicit local selection
        // command, never emitted as capture diagnostics or error log material.
        for device in capture::list_devices()? {
            println!(
                "{:?}\tdefault={}\t{}\t{}",
                device.source, device.is_default, device.name, device.id
            );
        }
        return Ok(());
    }
    let Command::Capture {
        microphone_id,
        render_id,
        duration,
        timings: timings_path,
    } = command
    else {
        unreachable!()
    };
    if timings_path.as_ref().is_some_and(|path| path.exists()) {
        return Err("probe_timings_exists".into());
    }
    let (sender, receiver) = queue::capture_event_queue();
    let stop = Arc::new(AtomicBool::new(false));
    let handle: capture::CaptureHandle = capture::start_capture(
        CaptureConfig {
            microphone_id,
            render_id,
            epoch: 0,
        },
        sender,
        stop.clone(),
    )?;
    let session_start = handle.start_qpc_100ns();
    let mut timings = timings_path
        .as_ref()
        .map(|_| TimingTrace::new(session_start));
    let started = Instant::now();
    let mut stats = [Statistics::default(), Statistics::default()];
    let mut error = None;
    while started.elapsed() < duration && !stop.load(Ordering::Acquire) {
        let remaining = duration.saturating_sub(started.elapsed());
        match receiver.recv_timeout(Duration::from_millis(100).min(remaining)) {
            Ok(CaptureEvent::Packet(packet)) => {
                if let Some(trace) = timings.as_mut() {
                    trace.packets.push(packet_timing(&packet));
                }
                let index = if packet.source == Source::Microphone {
                    0
                } else {
                    1
                };
                stats[index].packet(packet);
            }
            Ok(CaptureEvent::Heartbeat { source, qpc_100ns }) => {
                if let Some(trace) = timings.as_mut() {
                    trace.heartbeat(source, qpc_100ns);
                }
                let index = if source == Source::Microphone { 0 } else { 1 };
                stats[index].heartbeat(qpc_100ns);
            }
            Ok(CaptureEvent::Error { code, .. }) => {
                error = Some(code);
                break;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                error = Some("probe_workers_disconnected".into());
                break;
            }
        }
    }
    let result = handle.stop();
    while let Ok(event) = receiver.try_recv() {
        match event {
            CaptureEvent::Packet(packet) => {
                if let Some(trace) = timings.as_mut() {
                    trace.packets.push(packet_timing(&packet));
                }
                let index = if packet.source == Source::Microphone {
                    0
                } else {
                    1
                };
                stats[index].packet(packet);
            }
            CaptureEvent::Error { code, .. } => {
                error.get_or_insert(code);
            }
            CaptureEvent::Heartbeat { source, qpc_100ns } => {
                if let Some(trace) = timings.as_mut() {
                    trace.heartbeat(source, qpc_100ns);
                }
                let index = if source == Source::Microphone { 0 } else { 1 };
                stats[index].heartbeat(qpc_100ns);
            }
        }
    }
    println!(
        "elapsed_seconds={:.3} start_qpc_100ns={session_start} end_qpc_100ns={}",
        started.elapsed().as_secs_f64(),
        result.as_ref().copied().unwrap_or(0)
    );
    stats[0].print("microphone");
    stats[1].print("remote");
    if let (Some((mic, _)), Some((remote, _))) = (stats[0].first_reliable, stats[1].first_reliable)
    {
        println!(
            "first_packet_offset_100ns={}",
            i128::from(mic) - i128::from(remote)
        );
    }
    let session_end = result?;
    let microphone_boundaries = stats[0].covers_boundaries(session_start, session_end);
    let remote_boundaries = stats[1].covers_boundaries(session_start, session_end);
    println!(
        "common_interval_seconds={:.6} microphone_boundaries={} remote_boundaries={}",
        (session_end - session_start) as f64 / 10_000_000.0,
        microphone_boundaries,
        remote_boundaries
    );
    if !microphone_boundaries || !remote_boundaries {
        return Err("probe_capture_boundary_missing".into());
    }
    if let (Some(trace), Some(path)) = (timings.as_mut(), timings_path.as_ref()) {
        trace.end_qpc_100ns = session_end;
        trace.write(path)?;
    }
    error.map_or(Ok(()), Err)
}
