"""Check clock alignment with synthetic PCM and immutable existing dependencies.

No Cargo, audio devices, playback, ASR, or personal recording access.
"""
from pathlib import Path
import argparse
import os
import shutil
import subprocess
import uuid


# This standalone consumer accepts only the probe's timing whitelist. Its packet
# payload is always synthetic silence; it never opens user audio or devices.
TIMING_REPLAY = r'''
use meetings::{alignment::AlignedMeeting, store::MeetingStore, types::*};
use serde::Deserialize;
use std::{fs::File, io::Read, path::Path};
const MAX_TIMING_BYTES: u64 = 16 * 1024 * 1024;
const MAX_TIMING_PACKETS: usize = 20_000;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct TimingFormat {
    sample_rate: u32, channels: u16, bits_per_sample: u16, valid_bits: u16,
    channel_mask: u32, encoding: AudioEncoding,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct TimingPacket {
    source: Source, epoch: u32, sequence: u64, device_frame_position: u64,
    qpc_100ns: u64, frames: u32, flags: u32, format: TimingFormat,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct TimingHeartbeat { source: Source, qpc_100ns: u64 }
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct TimingInput {
    schema_version: u32, start_qpc_100ns: u64, end_qpc_100ns: u64,
    packets: Vec<TimingPacket>, heartbeats: Vec<TimingHeartbeat>,
}
#[derive(Debug)]
struct ReplaySummary { frames: u64, valid_samples: u64, first_qpc_100ns: u64, last_qpc_100ns: u64 }

impl TimingInput {
    fn validate(&self) -> Result<u32, String> {
        if self.schema_version != 1 || self.start_qpc_100ns == 0
            || self.end_qpc_100ns <= self.start_qpc_100ns
            // A probe requests at most 30s; allow bounded stop/scheduler latency.
            || self.end_qpc_100ns - self.start_qpc_100ns > 350_000_000 {
            return Err("timings_common_interval".into());
        }
        if self.packets.len() > MAX_TIMING_PACKETS || self.heartbeats.len() > MAX_TIMING_PACKETS {
            return Err("timings_count_limit".into());
        }
        // Native probe capture has one fixed epoch. Heartbeats have no epoch
        // field, so they must never be reused to certify several epochs.
        let epoch = self.packets.first().map(|p| p.epoch).unwrap_or(0);
        if self.packets.iter().any(|p| p.epoch != epoch) {
            return Err("timings_multiple_epochs".into());
        }
        for source in [Source::Microphone, Source::Remote] {
            let mut previous = None;
            for heartbeat in self.heartbeats.iter().filter(|h| h.source == source) {
                let observed = heartbeat.qpc_100ns;
                if observed < self.start_qpc_100ns || observed > self.end_qpc_100ns {
                    return Err("timings_health_interval".into());
                }
                if let Some(last) = previous {
                    if observed < last || observed - last > 15_000_000 {
                        return Err("timings_health_gap".into());
                    }
                } else if observed != self.start_qpc_100ns {
                    return Err("timings_health_boundary".into());
                }
                previous = Some(observed);
            }
            if previous != Some(self.end_qpc_100ns) {
                return Err("timings_health_boundary".into());
            }
        }
        Ok(epoch)
    }
}

fn read_timings(path: &Path) -> Result<TimingInput, String> {
    let file = File::open(path).map_err(|_| "timings_read")?;
    if !file.metadata().map_err(|_| "timings_read")?.is_file() {
        return Err("timings_file".into());
    }
    let mut bytes = vec![];
    file.take(MAX_TIMING_BYTES + 1).read_to_end(&mut bytes).map_err(|_| "timings_read")?;
    if bytes.len() as u64 > MAX_TIMING_BYTES {
        return Err("timings_byte_limit".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| "timings_schema".into())
}

fn replay(input: TimingInput, root: &Path) -> Result<ReplaySummary, String> {
    let epoch = input.validate()?;
    let packet_count = input.packets.len();
    let store = MeetingStore::new(root).map_err(str::to_owned)?;
    let mut writer = store.create("timing-replay", "Native timestamps with synthetic silence",
        input.start_qpc_100ns, vec![]).map_err(str::to_owned)?;
    for packet in input.packets {
        let format = packet.format;
        writer.append(CapturePacket {
            source: packet.source, epoch: packet.epoch, sequence: packet.sequence,
            device_frame_position: packet.device_frame_position, qpc_100ns: packet.qpc_100ns,
            frames: packet.frames,
            // Preserve every native flag, including errors and unknown bits.
            // SILENT changes only the payload; it cannot hide clock failures.
            flags: packet.flags | 2,
            format: AudioFormat {sample_rate: format.sample_rate, channels: format.channels,
                bits_per_sample: format.bits_per_sample, valid_bits: format.valid_bits,
                channel_mask: format.channel_mask, encoding: format.encoding},
            data: vec![],
        }).map_err(str::to_owned)?;
    }
    for heartbeat in input.heartbeats {
        writer.heartbeat(heartbeat.source, epoch, heartbeat.qpc_100ns).map_err(str::to_owned)?;
    }
    writer.finish(MeetingStatus::Recorded, Some(input.end_qpc_100ns), None).map_err(str::to_owned)?;
    let mut aligned = AlignedMeeting::open(store, "timing-replay")?;
    let warnings = aligned.warnings().len();
    let mut summary = ReplaySummary {frames: 0, valid_samples: 0, first_qpc_100ns: 0, last_qpc_100ns: 0};
    while let Some(frame) = aligned.next_frame()? {
        if frame.valid_samples == 0 || frame.valid_samples > 160
            || frame.microphone.iter().chain(frame.remote.iter()).any(|x| !x.is_finite() || *x != 0.0) {
            return Err("timings_nonzero_or_invalid_frame".into());
        }
        if frame.start_qpc_100ns < input.start_qpc_100ns
            || frame.start_qpc_100ns >= input.end_qpc_100ns
            // valid_samples counts sample timestamps before the exact native
            // stop, even when its final 62.5us sample cell crosses that stop.
            || frame.start_qpc_100ns + u64::from(frame.valid_samples - 1) * 625 >= input.end_qpc_100ns {
            return Err("timings_output_interval".into());
        }
        if summary.frames == 0 {summary.first_qpc_100ns = frame.start_qpc_100ns;}
        summary.frames += 1;
        summary.valid_samples += u64::from(frame.valid_samples);
        summary.last_qpc_100ns = frame.start_qpc_100ns;
    }
    if summary.frames == 0 {return Err("timings_no_aligned_frames".into());}
    println!("mode=native_timestamps+synthetic_silence packets={packet_count} frames={} valid_samples={} all_zero=true start_qpc_100ns={} end_qpc_100ns={} first_frame_qpc_100ns={} last_frame_qpc_100ns={} warnings={warnings}",
        summary.frames, summary.valid_samples, input.start_qpc_100ns, input.end_qpc_100ns,
        summary.first_qpc_100ns, summary.last_qpc_100ns);
    Ok(summary)
}

fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let result = if args.len() != 2 {Err("usage: meeting-timing-replay <timings-json> <new-synthetic-store>".into())}
        else {read_timings(Path::new(&args[0])).and_then(|input| replay(input, Path::new(&args[1])))};
    if let Err(error) = result {eprintln!("{error}"); std::process::exit(1);}
}

#[cfg(test)]
mod timing_replay_tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    fn fixture_value() -> serde_json::Value {
        let packets: Vec<_> = (0..100u64).flat_map(|i| ["microphone", "remote"].map(move |source| serde_json::json!({
            "source": source, "epoch": 0, "sequence": i,
            "device_frame_position": i * 480, "qpc_100ns": 1_000_000 + i * 100_000,
            "frames": 480, "flags": if i == 0 {1} else {0},
            "format": {"sample_rate": 48_000, "channels": 2, "bits_per_sample": 32,
                "valid_bits": 32, "channel_mask": 3, "encoding": "float"}
        }))).collect();
        serde_json::json!({"schema_version": 1, "start_qpc_100ns": 1_000_000,
            "end_qpc_100ns": 11_000_000, "packets": packets,
            "heartbeats": [
                {"source": "microphone", "qpc_100ns": 1_000_000},
                {"source": "remote", "qpc_100ns": 1_000_000},
                {"source": "microphone", "qpc_100ns": 11_000_000},
                {"source": "remote", "qpc_100ns": 11_000_000}]})
    }
    fn fixture() -> TimingInput { serde_json::from_value(fixture_value()).unwrap() }
    fn run(input: TimingInput) -> Result<ReplaySummary, String> {
        let root = std::env::current_dir().unwrap().join("outputs").join(format!(
            "timing-replay-test-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        let result = replay(input, &root);
        let _ = std::fs::remove_dir_all(root);
        result
    }
    #[test]
    fn timing_replay_preserves_common_interval_and_yields_only_synthetic_zero() {
        let summary = run(fixture()).unwrap();
        assert_eq!(summary.frames, 100);
        assert_eq!(summary.valid_samples, 16_000);
        assert_eq!(summary.first_qpc_100ns, 1_000_000);
        assert_eq!(summary.last_qpc_100ns, 10_900_000);
    }
    #[test]
    fn timing_replay_preserves_fractional_native_stop_without_extending_sample_timestamps() {
        let mut input = fixture();
        input.end_qpc_100ns = 10_999_700;
        input.heartbeats[2].qpc_100ns = 10_999_700;
        input.heartbeats[3].qpc_100ns = 10_999_700;
        let summary = run(input).unwrap();
        assert_eq!(summary.frames, 100);
        assert_eq!(summary.valid_samples, 16_000);
        assert_eq!(summary.last_qpc_100ns, 10_900_000);
    }
    #[test]
    fn timing_replay_does_not_hide_unknown_flags_timestamp_errors_or_clock_regression() {
        for (flags, expected) in [(8, "meeting_alignment_flags"), (4, "meeting_alignment_timestamp_error"), (1, "meeting_alignment_discontinuity")] {
            let mut input = fixture();
            input.packets[26].flags = flags;
            assert_eq!(run(input).unwrap_err(), expected);
        }
        let mut input = fixture();
        input.packets[26].qpc_100ns -= 100_001;
        assert_eq!(run(input).unwrap_err(), "meeting_alignment_qpc_regression");
        let mut input = fixture();
        input.packets[26].device_frame_position += 1;
        assert_eq!(run(input).unwrap_err(), "meeting_alignment_unknown_gap");
    }
    #[test]
    fn timing_replay_rejects_content_and_device_fields_at_every_schema_level() {
        let mut value = fixture_value();
        value["packets"][0]["data"] = serde_json::json!("PrivateSentinel");
        assert!(serde_json::from_value::<TimingInput>(value).is_err());
        let mut value = fixture_value();
        value["packets"][0]["format"]["name"] = serde_json::json!("PrivateSentinel");
        assert!(serde_json::from_value::<TimingInput>(value).is_err());
        let mut value = fixture_value();
        value["heartbeats"][0]["device_id"] = serde_json::json!("PrivateSentinel");
        assert!(serde_json::from_value::<TimingInput>(value).is_err());
        let mut value = fixture_value();
        value["content"] = serde_json::json!("PrivateSentinel");
        assert!(serde_json::from_value::<TimingInput>(value).is_err());
    }
    #[test]
    fn timing_replay_cannot_extend_heartbeat_evidence_or_accept_unbounded_metadata() {
        let mut input = fixture();
        input.heartbeats[0].qpc_100ns += 1;
        assert_eq!(run(input).unwrap_err(), "timings_health_boundary");
        let mut input = fixture();
        input.heartbeats.pop();
        assert_eq!(run(input).unwrap_err(), "timings_health_boundary");
        let mut input = fixture();
        input.end_qpc_100ns = 20_000_000;
        input.heartbeats[2].qpc_100ns = 20_000_000;
        input.heartbeats[3].qpc_100ns = 20_000_000;
        assert_eq!(run(input).unwrap_err(), "timings_health_gap");
        let mut input = fixture();
        input.packets = vec![input.packets[0].clone(); 20_001];
        assert_eq!(run(input).unwrap_err(), "timings_count_limit");
        let mut input = fixture();
        input.packets[0].epoch = 1;
        assert_eq!(run(input).unwrap_err(), "timings_multiple_epochs");
    }
}
'''


def main():
    root = Path(__file__).resolve().parents[1]
    target = Path(os.environ.get("CARGO_TARGET_DIR", "target"))
    if not target.is_absolute():
        target = root / "desktop/src-tauri" / target
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--deps", type=Path, default=target / "debug/deps", help="Read-only Cargo debug dependencies; prepare them with cargo test --no-run first")
    parser.add_argument("--filter", default="", help="Optional Rust test name filter")
    parser.add_argument("--timings", type=Path, help="Explicit metadata-only probe JSON to replay with synthetic silence after the pure tests")
    parser.add_argument("--output-dir", type=Path, default=root / "outputs/meeting-alignment-check", help="Isolated build directory inside this checkout's ignored outputs/")
    args = parser.parse_args()
    args.deps = args.deps.resolve()
    out = args.output_dir.resolve()
    if not out.is_relative_to((root / "outputs").resolve()):
        parser.error("--output-dir must be inside this checkout's ignored outputs/")
    out.mkdir(parents=True, exist_ok=True)
    libs = {}
    for name in ("serde", "serde_json", "rubato"):
        candidates = sorted(args.deps.glob(f"lib{name}-*.rlib"), key=lambda p: p.stat().st_mtime, reverse=True)
        if not candidates:
            raise SystemExit(f"Missing immutable {name} artifacts in {args.deps}")
        libs[name] = candidates[0]
    types = (root / "desktop/src-tauri/src/meetings/types.rs").read_text(encoding="utf-8").replace(", specta::Type", "")
    (out / "types.rs").write_text(types, encoding="utf-8")
    module_root = root / "desktop/src-tauri/src/meetings"
    harness = out / "tests.rs"
    harness.write_text(f'#![allow(dead_code)]\nmod meetings {{\n#[path = "{(out / "types.rs").as_posix()}"] pub(crate) mod types;\n' + ''.join(
        f'#[path = "{(module_root / (name + ".rs")).as_posix()}"] pub(crate) mod {name};\n'
        for name in ("queue", "store", "alignment")) + '}\n' + TIMING_REPLAY, encoding="utf-8")
    executable = out / "meeting-alignment-tests.exe"
    command = [shutil.which("rustc") or str(Path.home() / ".cargo/bin/rustc.exe"), "--edition=2021", "--test", "-O", str(harness), "-L", f"dependency={args.deps}", "-o", str(executable)]
    for name, path in libs.items():
        command += ["--extern", f"{name}={path}"]
    subprocess.run(command, check=True, cwd=root)
    result = subprocess.run([str(executable), args.filter, "--test-threads=1", "--nocapture"], cwd=root)
    if result.returncode or args.timings is None:
        return result.returncode
    replay_executable = out / "meeting-timing-replay.exe"
    replay_command = [item for item in command if item != "--test"]
    replay_command[replay_command.index("-o") + 1] = str(replay_executable)
    subprocess.run(replay_command, check=True, cwd=root)
    isolated_store = out / f"replay-{uuid.uuid4().hex}"
    return subprocess.run([str(replay_executable), str(args.timings.resolve()), str(isolated_store)], cwd=root).returncode


if __name__ == "__main__":
    raise SystemExit(main())
