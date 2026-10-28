//! Standalone native capture probe. No audio starts without explicit `capture`.
//!
//! Build with rustc and the existing application's serde/specta/windows rlibs.
//! `list` enumerates endpoints only. `capture <mic-id> <render-id> <seconds>`
//! requires both exact IDs and 1..=30 seconds, keeps statistics only and drops
//! every packet immediately. It never saves samples, plays audio or transcribes.
#[allow(dead_code, unused_imports)]
#[path = "../desktop/src-tauri/src/meetings/capture.rs"]
mod capture;
#[allow(dead_code)]
#[path = "../desktop/src-tauri/src/meetings/queue.rs"]
mod queue;
#[allow(dead_code)]
#[path = "../desktop/src-tauri/src/meetings/types.rs"]
mod types;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::Arc;
use std::time::{Duration, Instant};
use types::{CaptureConfig, CaptureEvent, CapturePacket, Source};

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
}

impl Statistics {
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
    if args.first().map(String::as_str) == Some("list") && args.len() == 1 {
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
    if args.len() != 4 || args[0] != "capture" {
        return Err("usage: probe-meeting-capture list | capture <microphone-id> <render-id> <seconds:1..30>".into());
    }
    let seconds: u64 = args[3].parse().map_err(|_| "probe_duration_invalid")?;
    if !(1..=30).contains(&seconds) {
        return Err("probe_duration_invalid".into());
    }
    let (sender, receiver) = queue::capture_event_queue();
    let stop = Arc::new(AtomicBool::new(false));
    let session_start = capture::now_qpc_100ns()?;
    let handle: capture::CaptureHandle = capture::start_capture(
        CaptureConfig {
            microphone_id: args[1].clone(),
            render_id: args[2].clone(),
            epoch: 0,
        },
        sender,
        stop.clone(),
    )?;
    let started = Instant::now();
    let mut stats = [Statistics::default(), Statistics::default()];
    let mut error = None;
    while started.elapsed() < Duration::from_secs(seconds) && !stop.load(Ordering::Acquire) {
        match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(CaptureEvent::Packet(packet)) => {
                let index = if packet.source == Source::Microphone {
                    0
                } else {
                    1
                };
                stats[index].packet(packet);
            }
            Ok(CaptureEvent::Heartbeat { .. }) => {}
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
            _ => {}
        }
    }
    println!(
        "elapsed_seconds={:.3} start_qpc_100ns={session_start} end_qpc_100ns={}",
        started.elapsed().as_secs_f64(),
        capture::now_qpc_100ns()?
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
    result?;
    error.map_or(Ok(()), Err)
}
