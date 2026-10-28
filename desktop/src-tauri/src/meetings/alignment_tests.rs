use super::super::store::MeetingWriter;
use super::*;
use std::{
    f64::consts::TAU,
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);
const BASE: u64 = 9_876_543_210;
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
                "alignment-synthetic-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        let store = MeetingStore::new(&root).unwrap();
        Self { root, store }
    }
    fn writer(&self) -> MeetingWriter {
        self.store
            .create("synthetic", "Synthetic", BASE, vec![])
            .unwrap()
    }
    fn open(&self) -> Result<AlignedMeeting, String> {
        AlignedMeeting::open(self.store.clone(), "synthetic")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn format(rate: u32, bits: u16, valid: u16, channels: u16, encoding: AudioEncoding) -> AudioFormat {
    AudioFormat {
        sample_rate: rate,
        channels,
        bits_per_sample: bits,
        valid_bits: valid,
        channel_mask: 0,
        encoding,
    }
}
fn pcm(format: AudioFormat, bytes: &[u8], frames: u32) -> CapturePacket {
    CapturePacket {
        source: Source::Microphone,
        epoch: 0,
        sequence: 0,
        device_frame_position: 0,
        qpc_100ns: BASE,
        frames,
        flags: 0,
        format,
        data: bytes.to_vec(),
    }
}

fn consensus_packet(rate: u32, ppm: f64, sequence: u64, noise: i64) -> CapturePacket {
    let frames = rate / 100;
    let position = sequence * u64::from(frames);
    let truth = (position as f64 * 1e7 / (rate as f64 * (1.0 + ppm * 1e-6))).round() as u64;
    let mut packet = pcm(format(rate, 32, 32, 1, AudioEncoding::Float), &[], frames);
    packet.flags = 2;
    packet.sequence = sequence;
    packet.device_frame_position = position;
    packet.qpc_100ns = (BASE + truth).checked_add_signed(noise).unwrap();
    packet
}

// The factory defines timing measurements only. The independent true sample
// clock used below never enters the estimator or its validator.
fn consensus_map(
    count: u64,
    factory: impl Fn(u64) -> CapturePacket,
) -> Result<(ClockMap, ClockValidationStats, bool), String> {
    let mut builder = Builder::new(&factory(0), 0);
    for sequence in 0..count {
        builder.push(&factory(sequence), sequence)?;
    }
    let segment = builder.finish()?;
    let mut validator = ClockValidator::new(&segment);
    for sequence in 0..count {
        validator.push(&factory(sequence))?;
    }
    Ok((
        segment.clock.clone(),
        validator.finish()?,
        segment.consensus_clock,
    ))
}

fn truth_phase_error_us(map: &ClockMap, rate: u32, ppm: f64, count: u64) -> f64 {
    let total = count * u64::from(rate / 100);
    let truth_rate = rate as f64 * (1.0 + ppm * 1e-6);
    let mut worst = 0.0f64;
    for position in (0..=total).step_by(17).chain(std::iter::once(total)) {
        let truth = BASE + (position as f64 * 1e7 / truth_rate).round() as u64;
        worst = worst.max(map.qpc(position as f64).unwrap().abs_diff(truth) as f64 / 10.0);
    }
    worst
}

fn timing_hash(mut value: u64) -> u64 {
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
    value ^ (value >> 31)
}

#[test]
fn clock_consensus_periodic_spikes_keep_independent_truth_with_all_phases_and_ppm() {
    let mut worst = 0.0f64;
    for rate in [44100, 48000] {
        for ppm in [-100.0, 100.0] {
            for sign in [-1i64, 1] {
                for phase in 0..12 {
                    let (map, stats, consensus) = consensus_map(1200, |sequence| {
                        let jitter = (timing_hash(sequence + 41) % 3001) as i64 - 1500;
                        let spike = if sequence % 12 == phase {
                            sign * 7500
                        } else {
                            0
                        };
                        consensus_packet(rate, ppm, sequence, jitter + spike)
                    })
                    .unwrap();
                    assert!(consensus);
                    assert!(stats.noisy_timestamps);
                    assert!(stats.max_buffered_anchors <= 32);
                    assert!(stats.max_pair_slopes <= 496);
                    let error = truth_phase_error_us(&map, rate, ppm, 1200);
                    worst = worst.max(error);
                    assert!(
                        error <= 250.0,
                        "{rate}Hz {ppm}ppm sign={sign} phase={phase}: {error}us"
                    );
                }
            }
        }
    }
    println!("fixed consensus periodic+random noise independent truth worst_us={worst}");
}

#[test]
fn clock_consensus_random_signed_pulses_and_partial_tails_keep_independent_truth() {
    let mut worst = 0.0f64;
    for count in [64, 65, 79, 95, 96, 97] {
        for rate in [44100, 48000] {
            for ppm in [-100.0, 100.0] {
                for seed in [7u64, 31, 101, 997] {
                    let (map, stats, consensus) = consensus_map(count, |sequence| {
                        let hash = timing_hash(sequence.wrapping_add(seed));
                        let jitter = (hash % 3001) as i64 - 1500;
                        let pulse = if sequence % 11 == seed % 11 {
                            if hash & 1 == 0 {
                                -7500
                            } else {
                                7500
                            }
                        } else {
                            0
                        };
                        consensus_packet(rate, ppm, sequence, jitter + pulse)
                    })
                    .unwrap();
                    assert!(consensus);
                    assert!(stats.max_buffered_anchors <= 32);
                    assert!(stats.max_pair_slopes <= 496);
                    let error = truth_phase_error_us(&map, rate, ppm, count);
                    worst = worst.max(error);
                    assert!(
                        error <= 250.0,
                        "count={count} {rate}Hz {ppm}ppm seed={seed}: {error}us"
                    );
                }
            }
        }
    }
    println!("fixed consensus random pulses/partial tails independent truth worst_us={worst}");
}

#[test]
fn clock_consensus_rejects_excess_uncertainty_contamination_offsets_and_warp() {
    for defect in 0..5 {
        let result = consensus_map(1200, |sequence| {
            let noise = match defect {
                0 if sequence == 600 => 12_000,
                1 if sequence % 32 < 9 => 7500,
                2 if sequence >= 600 => 10_000,
                3 if sequence >= 1168 => 10_000,
                4 if sequence >= 600 => ((sequence - 600) * 100) as i64,
                _ => 0,
            };
            consensus_packet(48000, 0.0, sequence, noise)
        });
        assert_eq!(
            result.err().as_deref(),
            Some("meeting_alignment_qpc_jitter"),
            "defect={defect}"
        );
    }
}

#[test]
fn clock_consensus_short_segments_keep_the_original_strict_anchor_rule() {
    for count in [2, 30, 32, 63] {
        let result = consensus_map(count, |sequence| {
            consensus_packet(
                16000,
                0.0,
                sequence,
                if sequence == count / 2 { 12_000 } else { 0 },
            )
        });
        assert_eq!(
            result.err().as_deref(),
            Some("meeting_alignment_qpc_jitter")
        );
    }
    let (_, stats, consensus) =
        consensus_map(63, |sequence| consensus_packet(48000, 100.0, sequence, 0)).unwrap();
    assert!(!consensus);
    assert!(!stats.noisy_timestamps);
}

#[test]
fn clock_consensus_does_not_hide_packet_integrity_errors_or_unknown_audio_gaps() {
    for (defect, expected) in [
        (0, "meeting_alignment_unknown_gap"),
        (1, "meeting_alignment_sequence"),
        (2, "meeting_alignment_timestamp_error"),
        (3, "meeting_alignment_discontinuity"),
        (4, "meeting_alignment_qpc_regression"),
        (5, "meeting_alignment_unknown_gap"),
        (6, "meeting_alignment_flags"),
    ] {
        let fixture = Fixture::new();
        let mut writer = fixture.writer();
        for sequence in 0..96 {
            for source in [Source::Microphone, Source::Remote] {
                let mut packet = consensus_packet(16000, 0.0, sequence, 0);
                packet.source = source;
                if source == Source::Microphone {
                    if sequence == 64 {
                        match defect {
                            0 => packet.device_frame_position += 160,
                            1 => packet.sequence += 1,
                            2 => packet.flags |= 4,
                            3 => packet.flags |= 1,
                            4 => packet.qpc_100ns -= 300_000,
                            6 => packet.flags |= 8,
                            _ => (),
                        }
                    }
                    if defect == 5 && sequence >= 64 {
                        packet.qpc_100ns += 30_000;
                    }
                }
                writer.append(packet).unwrap();
            }
        }
        writer.finish(MeetingStatus::Recorded, None, None).unwrap();
        assert_eq!(
            fixture.open().err().as_deref(),
            Some(expected),
            "defect={defect}"
        );
    }
}

#[test]
fn clock_consensus_two_hour_metadata_has_bounded_actual_working_buffers_and_phase_error() {
    let (map, stats, consensus) = consensus_map(720_000, |sequence| {
        let noise = if sequence % 12 == 3 { -7500 } else { 0 };
        consensus_packet(48000, 100.0, sequence, noise)
    })
    .unwrap();
    assert!(consensus);
    let error = truth_phase_error_us(&map, 48000, 100.0, 720_000);
    assert!(error <= 250.0);
    assert_eq!(stats.max_buffered_anchors, 32);
    assert_eq!(stats.max_pair_slopes, 496);
    println!("accelerated 2h metadata only: phase_error_us={error}, actual max anchors={}, pair slopes={}; no 2h audio or elapsed soak", stats.max_buffered_anchors, stats.max_pair_slopes);
}
// These literal edge cases catch signedness, little-endian and left-aligned valid-bit mistakes.
#[test]
fn literal_native_pcm_decodes_without_signedness_or_valid_bit_errors() {
    for (bits, valid, bytes, expected) in [
        (8, 8, vec![0, 128, 255], vec![-1.0, 0.0, 127.0 / 128.0]),
        (
            16,
            16,
            vec![0, 128, 0, 0, 255, 127],
            vec![-1.0, 0.0, 32767.0 / 32768.0],
        ),
        (
            24,
            24,
            vec![0, 0, 128, 0, 0, 0, 255, 255, 127],
            vec![-1.0, 0.0, 8388607.0 / 8388608.0],
        ),
        (
            32,
            24,
            vec![0, 0, 0, 128, 0, 0, 0, 0, 0, 255, 255, 127],
            vec![-1.0, 0.0, 8388607.0 / 8388608.0],
        ),
        (
            32,
            32,
            vec![0, 0, 0, 128, 0, 0, 0, 0, 255, 255, 255, 127],
            vec![-1.0, 0.0, 1.0],
        ),
        (
            16,
            12,
            vec![15, 128, 7, 0, 255, 127],
            vec![-1.0, 0.0, 2047.0 / 2048.0],
        ),
        (8, 4, vec![15, 143, 255], vec![-1.0, 0.0, 7.0 / 8.0]),
    ] {
        let p = pcm(format(48000, bits, valid, 1, AudioEncoding::Pcm), &bytes, 3);
        for (i, want) in expected.iter().enumerate() {
            assert!(
                (decode_mono(&p, i).unwrap() - want).abs() < 1e-7,
                "{bits}/{valid} sample {i}"
            );
        }
    }
}
#[test]
fn float_channels_mix_in_wide_accumulator_and_silent_packets_ignore_payload() {
    let mut p = pcm(
        format(48000, 32, 32, 2, AudioEncoding::Float),
        &[2f32.to_le_bytes(), (-1f32).to_le_bytes()].concat(),
        1,
    );
    assert_eq!(decode_mono(&p, 0).unwrap(), 0.5);
    p.data = [f32::MAX.to_le_bytes(), f32::MAX.to_le_bytes()].concat();
    assert_eq!(decode_mono(&p, 0).unwrap(), f32::MAX);
    p.data = [f32::NAN.to_le_bytes(), 0f32.to_le_bytes()].concat();
    assert!(decode_mono(&p, 0).is_err());
    p.flags = 2;
    p.data.clear();
    assert_eq!(decode_mono(&p, 0).unwrap(), 0.0);
    p.flags = 0;
    assert!(decode_mono(&p, 0).is_err());
}
fn signal(t: f64) -> f32 {
    (0.32 * (TAU * 437.0 * t).sin()
        + 0.17 * (TAU * 1103.0 * t + 0.37).sin()
        + 0.08 * (TAU * 2579.0 * t + 0.61).sin()) as f32
}
#[allow(clippy::too_many_arguments)]
fn append_signal(
    w: &mut MeetingWriter,
    source: Source,
    epoch: u32,
    start: u64,
    seconds: f64,
    rate: u32,
    ppm: f64,
    packet_sizes: &[u32],
    jitter: bool,
    tone: Option<f64>,
) {
    let actual = rate as f64 * (1.0 + ppm * 1e-6);
    let total = (seconds * actual).round() as u64;
    let mut pos = 0;
    let mut seq = 0;
    while pos < total {
        let frames =
            (packet_sizes[seq as usize % packet_sizes.len()] as u64).min(total - pos) as u32;
        let noise = if jitter {
            [0i64, 1500, -1200, 700, -900, 0, 900, -700, 1200, -1500][seq as usize % 10]
        } else {
            0
        };
        let time = (pos as f64 * 10_000_000.0 / actual).round() as u64;
        let data = (0..frames)
            .flat_map(|i| {
                let t = (pos + i as u64) as f64 / actual;
                let value = tone
                    .map(|f| (0.5 * (TAU * f * t).sin()) as f32)
                    .unwrap_or_else(|| signal(t));
                value.to_le_bytes()
            })
            .collect();
        w.append(CapturePacket {
            source,
            epoch,
            sequence: seq,
            device_frame_position: pos,
            qpc_100ns: (start + time).checked_add_signed(noise).unwrap(),
            frames,
            flags: if seq == 0 { 1 } else { 0 },
            format: format(rate, 32, 32, 1, AudioEncoding::Float),
            data,
        })
        .unwrap();
        pos += frames as u64;
        seq += 1;
    }
}
fn collect(mut reader: AlignedMeeting) -> Vec<AlignedFrame> {
    let mut frames = vec![];
    while let Some(frame) = reader.next_frame().unwrap() {
        frames.push(frame);
    }
    frames
}

#[test]
fn independent_48k_44k1_clocks_with_ppm_and_qpc_jitter_stay_in_phase_across_storage_chunks() {
    let f = Fixture::new();
    let mut w = f.writer();
    append_signal(
        &mut w,
        Source::Microphone,
        0,
        BASE,
        12.0,
        48000,
        100.0,
        &[480, 379, 641, 211],
        true,
        None,
    );
    append_signal(
        &mut w,
        Source::Remote,
        0,
        BASE,
        12.0,
        44100,
        -100.0,
        &[441, 553, 227, 704],
        true,
        None,
    );
    w.finish(MeetingStatus::Recorded, Some(BASE + 120_000_000), None)
        .unwrap();
    let reader = f.open().unwrap();
    assert!(reader
        .warnings()
        .iter()
        .any(|x| x.code == "meeting_alignment_initial_discontinuity"));
    let frames = collect(reader);
    assert!((1199..=1201).contains(&frames.len()));
    let mut squared = [0f64; 3];
    let mut count = 0;
    let mut boundary_squared = [0f64; 2];
    let mut boundary_count = 0;
    for frame in frames {
        for i in 0..160 {
            let t = (frame.start_qpc_100ns - BASE + i as u64 * 625) as f64 / 10_000_000.0;
            if !(0.02..11.98).contains(&t) {
                continue;
            }
            let expected = signal(t) as f64;
            let m = frame.microphone[i] as f64;
            let r = frame.remote[i] as f64;
            squared[0] += (m - expected).powi(2);
            squared[1] += (r - expected).powi(2);
            squared[2] += (m - r).powi(2);
            count += 1;
            if (t - 5.0).abs() < 0.02 || (t - 10.0).abs() < 0.02 {
                boundary_squared[0] += (m - expected).powi(2);
                boundary_squared[1] += (r - expected).powi(2);
                boundary_count += 1;
            }
        }
    }
    let rms = squared.map(|x| (x / count as f64).sqrt());
    println!("independent 12s drift+jitter RMS mic/remote/pair={rms:?}");
    assert!(
        rms.iter().all(|x| *x < 0.008),
        "QPC mapped analytic signal: {rms:?}"
    );
    let boundary_rms = boundary_squared.map(|x| (x / boundary_count as f64).sqrt());
    println!("5s/10s storage boundary analytic RMS mic/remote={boundary_rms:?}");
    assert!(
        boundary_rms.iter().all(|x| *x < 0.001),
        "storage chunk boundary phase: {boundary_rms:?}"
    );
}
#[test]
fn fractional_start_offsets_are_aligned_to_qpc_instead_of_integer_sample_indices() {
    let f = Fixture::new();
    let mut w = f.writer();
    append_signal(
        &mut w,
        Source::Microphone,
        0,
        BASE,
        1.0,
        48000,
        0.0,
        &[480],
        false,
        None,
    );
    // Remote begins 0.1234ms later and has the corresponding physical signal phase.
    let shift = 1234u64;
    let actual = 44100f64;
    for seq in 0..100u64 {
        let pos = seq * 441;
        let data = (0..441u64)
            .flat_map(|i| signal(shift as f64 / 1e7 + (pos + i) as f64 / actual).to_le_bytes())
            .collect();
        w.append(CapturePacket {
            source: Source::Remote,
            epoch: 0,
            sequence: seq,
            device_frame_position: pos,
            qpc_100ns: BASE + shift + seq * 100_000,
            frames: 441,
            flags: 0,
            format: format(44100, 32, 32, 1, AudioEncoding::Float),
            data,
        })
        .unwrap();
    }
    w.heartbeat(Source::Remote, 0, BASE).unwrap();
    w.heartbeat(Source::Remote, 0, BASE + 10_001_234).unwrap();
    w.finish(MeetingStatus::Recorded, Some(BASE + 10_001_234), None)
        .unwrap();
    let frames = collect(f.open().unwrap());
    let mut err = 0f64;
    let mut count = 0;
    for frame in frames {
        for i in 0..160 {
            let t = (frame.start_qpc_100ns - BASE + i as u64 * 625) as f64 / 1e7;
            if (0.02..0.98).contains(&t) {
                err += (frame.microphone[i] - frame.remote[i]).abs() as f64;
                count += 1;
            }
        }
    }
    assert!(
        err / (count as f64) < 0.002,
        "fractional QPC phase MAE={}",
        err / count as f64
    );
}
#[test]
fn bandlimited_downsampling_rejects_above_nyquist_tone() {
    let f = Fixture::new();
    let mut w = f.writer();
    append_signal(
        &mut w,
        Source::Microphone,
        0,
        BASE,
        1.0,
        48000,
        0.0,
        &[479, 481],
        false,
        Some(12000.0),
    );
    w.heartbeat(Source::Remote, 0, BASE).unwrap();
    w.heartbeat(Source::Remote, 0, BASE + 10_000_000).unwrap();
    w.finish(MeetingStatus::Recorded, Some(BASE + 10_000_000), None)
        .unwrap();
    let frames = collect(f.open().unwrap());
    let mut energy = 0f64;
    let mut n = 0;
    for frame in &frames[2..frames.len() - 2] {
        for sample in frame.microphone {
            energy += (sample as f64).powi(2);
            n += 1;
        }
    }
    let rms = (energy / n as f64).sqrt();
    println!("12kHz alias RMS={rms}");
    assert!(rms < 0.00035);
}
#[test]
fn remote_without_packets_requires_heartbeat_coverage_and_epochs_jump_over_pause() {
    let f = Fixture::new();
    let mut w = f.writer();
    for (epoch, start) in [(0, BASE), (1, BASE + 90_000_000)] {
        append_signal(
            &mut w,
            Source::Microphone,
            epoch,
            start,
            0.1,
            16000,
            0.0,
            &[160],
            false,
            None,
        );
        w.heartbeat(Source::Remote, epoch, start).unwrap();
        w.heartbeat(Source::Remote, epoch, start + 1_000_000)
            .unwrap();
    }
    w.finish(MeetingStatus::Recorded, Some(BASE + 91_000_000), None)
        .unwrap();
    let frames = collect(f.open().unwrap());
    assert_eq!(frames.len(), 20);
    assert_eq!(frames[10].epoch, 1);
    assert_eq!(frames[10].start_qpc_100ns, BASE + 90_000_000);
    assert!(frames.iter().all(|x| x.remote == [0.0; 160]));
    let bad = Fixture::new();
    let mut w = bad.writer();
    append_signal(
        &mut w,
        Source::Microphone,
        0,
        BASE,
        0.1,
        16000,
        0.0,
        &[160],
        false,
        None,
    );
    w.finish(MeetingStatus::Recorded, None, None).unwrap();
    assert!(bad.open().is_err());
}
#[test]
fn timestamp_errors_midstream_discontinuities_and_unknown_microphone_gaps_fail_without_file_changes(
) {
    for kind in ["timestamp", "discontinuity", "position", "qpc", "sequence"] {
        let f = Fixture::new();
        let mut w = f.writer();
        let mut a = pcm(format(16000, 16, 16, 1, AudioEncoding::Pcm), &[0; 320], 160);
        w.append(a.clone()).unwrap();
        a.sequence = 1;
        a.device_frame_position = 160;
        a.qpc_100ns = BASE + 100_000;
        match kind {
            "timestamp" => a.flags = 4,
            "discontinuity" => a.flags = 1,
            "position" => a.device_frame_position = 480,
            "qpc" => a.qpc_100ns = BASE - 1,
            "sequence" => a.sequence = 3,
            _ => unreachable!(),
        }
        w.append(a).unwrap();
        w.heartbeat(Source::Remote, 0, BASE).unwrap();
        w.heartbeat(Source::Remote, 0, BASE + 300_000).unwrap();
        w.finish(MeetingStatus::Recorded, None, None).unwrap();
        let manifest_path = f.root.join("synthetic/manifest.json");
        let before = fs::read(&manifest_path).unwrap();
        let error = f.open().err().expect(kind);
        assert!(error.starts_with("meeting_alignment_"), "{kind}: {error}");
        assert_eq!(fs::read(manifest_path).unwrap(), before);
    }
}

#[test]
fn remote_clock_restart_uses_verified_no_playback_gap_and_preserves_following_physical_phase() {
    let f = Fixture::new();
    let mut w = f.writer();
    append_signal(
        &mut w,
        Source::Microphone,
        0,
        BASE,
        0.3,
        48000,
        0.0,
        &[480],
        false,
        None,
    );
    for seq in 0..20u64 {
        let packet_index = if seq < 10 { seq } else { seq + 10 };
        let physical_time = packet_index as f64 * 0.01;
        let data = (0..441)
            .flat_map(|i| signal(physical_time + i as f64 / 44100.0).to_le_bytes())
            .collect();
        w.append(CapturePacket {
            source: Source::Remote,
            epoch: 0,
            sequence: seq,
            device_frame_position: (seq % 10) * 441,
            qpc_100ns: BASE + packet_index * 100_000,
            frames: 441,
            flags: if seq == 10 { 1 } else { 0 },
            format: format(44100, 32, 32, 1, AudioEncoding::Float),
            data,
        })
        .unwrap();
    }
    w.heartbeat(Source::Remote, 0, BASE).unwrap();
    w.heartbeat(Source::Remote, 0, BASE + 3_000_000).unwrap();
    w.finish(MeetingStatus::Recorded, Some(BASE + 3_000_000), None)
        .unwrap();
    let reader = f.open().unwrap();
    assert!(reader
        .warnings()
        .iter()
        .any(|x| x.code == "meeting_alignment_remote_resume_discontinuity"));
    let frames = collect(reader);
    assert!(frames[10..20].iter().all(|x| x.remote == [0.0; 160]));
    let mut err = 0f64;
    let mut n = 0;
    for frame in &frames[22..28] {
        for i in 0..160 {
            err += (frame.microphone[i] - frame.remote[i]).abs() as f64;
            n += 1;
        }
    }
    assert!(
        err / (n as f64) < 0.002,
        "restart phase MAE={}",
        err / n as f64
    );
}

#[test]
fn partial_frame_exposes_exact_valid_samples_without_extending_epoch() {
    let f = Fixture::new();
    let mut w = f.writer();
    let mut p = pcm(format(16000, 16, 16, 1, AudioEncoding::Pcm), &[], 177);
    p.flags = 2;
    w.append(p).unwrap();
    w.heartbeat(Source::Remote, 0, BASE).unwrap();
    w.heartbeat(Source::Remote, 0, BASE + 110_625).unwrap();
    w.finish(MeetingStatus::Recorded, None, None).unwrap();
    let reader = f.open().unwrap();
    assert!(reader
        .warnings()
        .iter()
        .any(|x| x.code == "meeting_alignment_single_anchor_nominal_rate"));
    let frames = collect(reader);
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0].valid_samples, 160);
    assert_eq!(frames[1].valid_samples, 17);
    assert_eq!(frames[1].start_qpc_100ns, BASE + 100_000);
    assert_eq!(frames[1].microphone, [0.0; 160]);
}

#[test]
fn a_missing_source_is_never_accepted_as_small_boundary_padding() {
    let f = Fixture::new();
    let mut w = f.writer();
    let mut p = pcm(format(16000, 16, 16, 1, AudioEncoding::Pcm), &[], 1);
    p.flags = 2;
    w.append(p).unwrap();
    w.finish(MeetingStatus::Recorded, None, None).unwrap();
    assert!(
        f.open().is_err(),
        "no remote heartbeat cannot establish healthy silence even for one sample"
    );
}

#[test]
fn two_hour_accelerated_clock_mapping_retains_ppm_precision_without_allocating_audio() {
    for (rate, ppm) in [(48000, 100.0), (44100, -100.0)] {
        let actual = rate as f64 * (1.0 + ppm * 1e-6);
        let mut regression = ClockRegression::default();
        for second in 0..=7200u64 {
            let position = (second as f64 * actual).round();
            let jitter = [0.0, 1500.0, -1500.0, 750.0, -750.0][second as usize % 5];
            regression.push(position, second as f64 * 1e7 + jitter);
        }
        let map = regression.finish(BASE, rate).unwrap();
        for seconds in [0.0f64, 1.0, 3599.1234, 7199.98, 7200.0] {
            let qpc = BASE + (seconds * 1e7).round() as u64;
            let error = (map.position(qpc) - seconds * actual).abs();
            assert!(error<0.01,"accelerated clock mapping {rate} {ppm}ppm at {seconds}s error {error} native samples");
        }
        println!("accelerated 2h metadata-only clock mapping: rate={rate} ppm={ppm}; no 2h audio or elapsed soak");
    }
}

#[test]
fn excessive_qpc_jitter_and_healthless_remote_clock_restart_are_rejected() {
    for healthy in [true, false] {
        let f = Fixture::new();
        let mut w = f.writer();
        append_signal(
            &mut w,
            Source::Microphone,
            0,
            BASE,
            0.3,
            16000,
            0.0,
            &[160],
            false,
            None,
        );
        for seq in 0..30u64 {
            let qpc = BASE
                + seq * 100_000
                + if healthy && seq == 15 {
                    12_000
                } else if !healthy && seq >= 15 {
                    100_000
                } else {
                    0
                };
            let mut p = pcm(format(16000, 16, 16, 1, AudioEncoding::Pcm), &[], 160);
            p.source = Source::Remote;
            p.sequence = seq;
            p.device_frame_position = seq * 160;
            p.qpc_100ns = qpc;
            p.flags = 2;
            w.append(p).unwrap();
        }
        if healthy {
            w.heartbeat(Source::Remote, 0, BASE).unwrap();
            w.heartbeat(Source::Remote, 0, BASE + 4_000_000).unwrap();
        }
        w.finish(MeetingStatus::Recorded, None, None).unwrap();
        let error = f.open().err().unwrap();
        assert_eq!(
            error,
            if healthy {
                "meeting_alignment_qpc_jitter"
            } else {
                "meeting_alignment_unknown_gap"
            }
        );
    }
}

#[test]
fn nonfinite_resampler_output_is_explicit_and_reader_failure_remains_terminal() {
    let f = Fixture::new();
    let mut w = f.writer();
    let data = (0..4800)
        .flat_map(|_| f32::MAX.to_le_bytes())
        .collect::<Vec<_>>();
    let p = pcm(format(48000, 32, 32, 1, AudioEncoding::Float), &data, 4800);
    w.append(p).unwrap();
    w.heartbeat(Source::Remote, 0, BASE).unwrap();
    w.heartbeat(Source::Remote, 0, BASE + 1_000_000).unwrap();
    w.finish(MeetingStatus::Recorded, None, None).unwrap();
    let mut reader = f.open().unwrap();
    let error = reader.next_frame().unwrap_err();
    assert_eq!(error, "meeting_alignment_sample_range");
    assert_eq!(reader.next_frame().unwrap_err(), error);
}

#[test]
fn streaming_windows_remain_bounded_and_a_read_failure_cannot_resume_half_a_frame() {
    let f = Fixture::new();
    let mut w = f.writer();
    append_signal(
        &mut w,
        Source::Microphone,
        0,
        BASE,
        6.0,
        48000,
        0.0,
        &[480],
        false,
        None,
    );
    w.heartbeat(Source::Remote, 0, BASE).unwrap();
    w.heartbeat(Source::Remote, 0, BASE + 60_000_000).unwrap();
    let manifest = w.finish(MeetingStatus::Recorded, None, None).unwrap();
    let mut reader = f.open().unwrap();
    let later = f.root.join("synthetic").join(&manifest.chunks[1].file);
    let hidden = later.with_extension("temporary");
    fs::rename(&later, &hidden).unwrap();
    let mut count = 0;
    let error = loop {
        match reader.next_frame() {
            Ok(Some(_)) => {
                count += 1;
                for track in &reader.tracks {
                    if let Some(window) = &track.window {
                        assert!(
                            window.cache.len() < 300,
                            "streaming FIR cache grew with packet count"
                        );
                    }
                }
            }
            Err(error) => break error,
            Ok(None) => panic!("missing later storage chunk must fail"),
        }
    };
    assert!((490..=500).contains(&count));
    fs::rename(hidden, later).unwrap();
    assert_eq!(
        reader.next_frame().unwrap_err(),
        error,
        "read failure is terminal; reopen to retry, never resume partial frame state"
    );
}

#[test]
fn short_anchor_spans_use_nominal_rate_with_traceable_uncertainty_and_do_not_fit_unknown_gaps() {
    for extra_ticks in [1500u64, 10_000] {
        let f = Fixture::new();
        let mut w = f.writer();
        for seq in 0..2 {
            let mut p = pcm(format(16000, 16, 16, 1, AudioEncoding::Pcm), &[], 160);
            p.flags = 2;
            p.sequence = seq;
            p.device_frame_position = seq * 160;
            p.qpc_100ns = BASE + seq * (100_000 + extra_ticks);
            w.append(p).unwrap();
        }
        w.heartbeat(Source::Remote, 0, BASE).unwrap();
        w.heartbeat(Source::Remote, 0, BASE + 300_000).unwrap();
        w.finish(MeetingStatus::Recorded, None, None).unwrap();
        if extra_ticks == 1500 {
            let reader = f.open().unwrap();
            assert!(reader
                .warnings()
                .iter()
                .any(|x| x.code == "meeting_alignment_short_span_nominal_rate"));
        } else {
            assert_eq!(
                f.open().err().as_deref(),
                Some("meeting_alignment_qpc_jitter"),
                "two anchors cannot turn a 1ms unknown hole into a 10% device clock change"
            );
        }
    }
}

#[test]
fn excessive_clock_restarts_fail_explicitly_before_metadata_can_grow_without_bound() {
    let mut p = pcm(format(16000, 16, 16, 1, AudioEncoding::Pcm), &[], 1);
    p.flags = 2;
    let mut segments = vec![];
    for ordinal in 0..20_000 {
        let mut builder = Builder::new(&p, ordinal);
        builder.push(&p, ordinal).unwrap();
        if let Err(error) = push_segment(&mut segments, builder) {
            assert_eq!(error, "meeting_alignment_segment_limit");
            assert!(segments.len() < 20_000);
            return;
        }
    }
    panic!("unlimited remote clock restart metadata must be rejected explicitly");
}

#[test]
fn recorded_heartbeat_only_meetings_explicitly_reject_missing_microphone_audio() {
    for heartbeats in [false, true] {
        let f = Fixture::new();
        let mut w = f.writer();
        if heartbeats {
            for source in [Source::Microphone, Source::Remote] {
                w.heartbeat(source, 0, BASE).unwrap();
                w.heartbeat(source, 0, BASE + 20_000_000).unwrap();
            }
        }
        w.finish(MeetingStatus::Recorded, Some(BASE + 20_000_000), None)
            .unwrap();
        assert_eq!(
            f.open().err().as_deref(),
            Some("meeting_alignment_missing_microphone_audio"),
            "empty recording must not become a successful empty transcript"
        );
    }
}

#[test]
fn remote_no_playback_gap_uses_previous_packet_rate_when_format_changes() {
    for (before, after) in [(48000, 16000), (16000, 48000), (96000, 384000)] {
        let f = Fixture::new();
        let mut w = f.writer();
        append_signal(
            &mut w,
            Source::Microphone,
            0,
            BASE,
            0.03,
            16000,
            0.0,
            &[160],
            false,
            None,
        );
        for (sequence, rate, qpc) in [(0, before, BASE), (1, after, BASE + 200_000)] {
            let frames = rate / 100;
            let data = (0..frames)
                .flat_map(|_| 0.25f32.to_le_bytes())
                .collect::<Vec<_>>();
            let mut p = pcm(format(rate, 32, 32, 1, AudioEncoding::Float), &data, frames);
            p.source = Source::Remote;
            p.sequence = sequence;
            p.qpc_100ns = qpc;
            p.flags = if sequence == 0 { 0 } else { 1 };
            w.append(p).unwrap();
        }
        w.heartbeat(Source::Remote, 0, BASE).unwrap();
        w.heartbeat(Source::Remote, 0, BASE + 300_000).unwrap();
        w.finish(MeetingStatus::Recorded, Some(BASE + 300_000), None)
            .unwrap();
        let reader = f.open().unwrap();
        let gaps = reader
            .warnings()
            .iter()
            .filter(|x| x.code == "meeting_alignment_remote_no_playback")
            .collect::<Vec<_>>();
        assert!(!gaps.is_empty());
        assert!(
            gaps.iter().all(|x| x.qpc_100ns == BASE + 100_000),
            "gap starts at the preceding packet's physical end for {before}->{after}"
        );
        let frames = collect(reader);
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[1].remote, [0.0; 160]);
        assert!((frames[2].remote[80] - 0.25).abs() < 0.01);
    }
}

#[test]
fn high_native_rates_keep_the_same_physical_antialias_transition_band() {
    for rate in [96000, 192000, 384000] {
        let f = Fixture::new();
        let mut w = f.writer();
        append_signal(
            &mut w,
            Source::Microphone,
            0,
            BASE,
            0.2,
            rate,
            0.0,
            &[rate / 100],
            false,
            Some(12000.0),
        );
        w.heartbeat(Source::Remote, 0, BASE).unwrap();
        w.heartbeat(Source::Remote, 0, BASE + 2_000_000).unwrap();
        w.finish(MeetingStatus::Recorded, Some(BASE + 2_000_000), None)
            .unwrap();
        let frames = collect(f.open().unwrap());
        let mut energy = 0.0f64;
        let mut count = 0;
        for frame in &frames[2..frames.len() - 2] {
            for sample in frame.microphone {
                energy += (sample as f64).powi(2);
                count += 1;
            }
        }
        let rms = (energy / count as f64).sqrt();
        println!("native {rate}Hz 12kHz alias RMS={rms}");
        assert!(
            rms < 0.00035,
            "unchanged independent anti-alias gate at native {rate}Hz: {rms}"
        );
    }
}

#[test]
fn mixed_384k_44k1_clocks_remain_analytic_and_streaming_with_ppm() {
    let f = Fixture::new();
    let mut w = f.writer();
    append_signal(
        &mut w,
        Source::Microphone,
        0,
        BASE,
        2.0,
        384000,
        100.0,
        &[3840, 2567, 4991],
        false,
        None,
    );
    append_signal(
        &mut w,
        Source::Remote,
        0,
        BASE,
        2.0,
        44100,
        -100.0,
        &[441, 311, 701],
        false,
        None,
    );
    w.finish(MeetingStatus::Recorded, Some(BASE + 20_000_000), None)
        .unwrap();
    let mut reader = f.open().unwrap();
    let mut squared = [0.0f64; 2];
    let mut count = 0;
    while let Some(frame) = reader.next_frame().unwrap() {
        for track in &reader.tracks {
            if let Some(window) = &track.window {
                assert!(
                    window.cache.len() <= 2050,
                    "native-rate-scaled FIR window remains bounded"
                );
            }
        }
        for i in 0..frame.valid_samples as usize {
            let time = (frame.start_qpc_100ns - BASE + i as u64 * 625) as f64 / 1e7;
            if !(0.02..1.98).contains(&time) {
                continue;
            }
            let expected = signal(time) as f64;
            squared[0] += (frame.microphone[i] as f64 - expected).powi(2);
            squared[1] += (frame.remote[i] as f64 - expected).powi(2);
            count += 1;
        }
    }
    let rms = squared.map(|x| (x / count as f64).sqrt());
    println!("384k/44.1k independent analytic RMS={rms:?}");
    assert!(rms.iter().all(|x| *x < 0.008));
}

#[test]
fn shared_manifest_interval_clips_startup_and_drain_audio_without_extending_remote_health() {
    let f = Fixture::new();
    let mut w = f.writer();
    let raw_start = BASE - 100_000;
    append_signal(
        &mut w,
        Source::Microphone,
        0,
        raw_start,
        0.05,
        48000,
        0.0,
        &[480],
        false,
        None,
    );
    let end = BASE + 210_625;
    w.heartbeat(Source::Remote, 0, BASE).unwrap();
    w.heartbeat(Source::Remote, 0, end).unwrap();
    let manifest = w.finish(MeetingStatus::Recorded, Some(end), None).unwrap();
    let originals = manifest
        .chunks
        .iter()
        .map(|c| {
            let path = f.root.join("synthetic").join(&c.file);
            let bytes = fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect::<Vec<_>>();
    let reader = f.open().unwrap();
    assert!(reader
        .warnings()
        .iter()
        .any(|x| x.code == "meeting_alignment_boundary_trimmed"));
    let frames = collect(reader);
    assert_eq!(frames.len(), 3);
    assert_eq!(frames[0].start_qpc_100ns, BASE);
    assert_eq!(frames[2].valid_samples, 17);
    assert!(frames.iter().all(|x| x.remote == [0.0; 160]));
    for (path, bytes) in originals {
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
}

#[test]
fn effective_common_interval_uses_real_microphone_start_and_does_not_invent_a_heartbeat_prefix() {
    let f = Fixture::new();
    let mut w = f.writer();
    append_signal(
        &mut w,
        Source::Microphone,
        0,
        BASE + 50_000,
        0.03,
        48000,
        0.0,
        &[480],
        false,
        None,
    );
    append_signal(
        &mut w,
        Source::Remote,
        0,
        BASE - 50_000,
        0.05,
        44100,
        0.0,
        &[441],
        false,
        None,
    );
    w.finish(MeetingStatus::Recorded, Some(BASE + 300_000), None)
        .unwrap();
    let reader = f.open().unwrap();
    assert!(reader
        .warnings()
        .iter()
        .any(|x| x.code == "meeting_alignment_boundary_trimmed"));
    let frames = collect(reader);
    assert_eq!(frames[0].start_qpc_100ns, BASE + 50_000);
    assert_eq!(frames.len(), 3);
    assert_eq!(frames[2].valid_samples, 80);

    let unknown = Fixture::new();
    let mut w = unknown.writer();
    append_signal(
        &mut w,
        Source::Microphone,
        0,
        BASE,
        0.1,
        16000,
        0.0,
        &[160],
        false,
        None,
    );
    // The old first periodic heartbeat cannot establish health before itself.
    w.heartbeat(Source::Remote, 0, BASE + 500_000).unwrap();
    w.heartbeat(Source::Remote, 0, BASE + 1_000_000).unwrap();
    w.finish(MeetingStatus::Recorded, Some(BASE + 1_000_000), None)
        .unwrap();
    assert_eq!(
        unknown.open().err().as_deref(),
        Some("meeting_alignment_source_unavailable")
    );
}

#[test]
fn fir_cannot_leak_tones_before_start_or_after_stop_into_the_recorded_interval() {
    let f = Fixture::new();
    let mut w = f.writer();
    for sequence in 0..6u64 {
        let outside = sequence == 0 || sequence >= 4;
        let data = (0..480u64)
            .flat_map(|sample| {
                let value = if outside {
                    0.75 * (TAU * 1200.0 * (sequence * 480 + sample) as f64 / 48000.0 + 0.7).sin()
                        as f32
                } else {
                    0.0
                };
                value.to_le_bytes()
            })
            .collect::<Vec<_>>();
        let mut packet = pcm(format(48000, 32, 32, 1, AudioEncoding::Float), &data, 480);
        packet.sequence = sequence;
        packet.device_frame_position = sequence * 480;
        packet.qpc_100ns = BASE - 100_000 + sequence * 100_000;
        w.append(packet).unwrap();
    }
    w.heartbeat(Source::Remote, 0, BASE).unwrap();
    w.heartbeat(Source::Remote, 0, BASE + 300_000).unwrap();
    w.finish(MeetingStatus::Recorded, Some(BASE + 300_000), None)
        .unwrap();
    let frames = collect(f.open().unwrap());
    assert_eq!(frames.len(), 3);
    assert!(
        frames.iter().all(|x| x.microphone == [0.0; 160]),
        "outside-boundary source audio cannot enter the centered FIR context"
    );
}
