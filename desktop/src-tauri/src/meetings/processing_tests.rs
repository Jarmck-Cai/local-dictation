use super::super::types::{AudioEncoding, AudioFormat, CapturePacket};
use super::*;
use std::{cell::RefCell, fs, path::PathBuf, rc::Rc, sync::atomic::AtomicU64};

struct Level;
impl Detection for Level {
    fn speech(&mut self, x: &[f32; VAD_SAMPLES]) -> Result<bool, String> {
        Ok(x.iter().any(|v| v.abs() > 0.05))
    }
    fn reset(&mut self) {}
}
struct Constant(bool);
impl Detection for Constant {
    fn speech(&mut self, _: &[f32; VAD_SAMPLES]) -> Result<bool, String> {
        Ok(self.0)
    }
    fn reset(&mut self) {}
}

struct OfficialV4Window {
    calls: usize,
    last_frame: Vec<f32>,
}
impl Detection for OfficialV4Window {
    fn speech(&mut self, frame: &[f32; VAD_SAMPLES]) -> Result<bool, String> {
        assert_eq!(frame.len(), 512, "official Silero v4's 16k minimum window");
        self.calls += 1;
        self.last_frame = frame.to_vec();
        Ok(true)
    }
    fn reset(&mut self) {
        self.calls = 0;
        self.last_frame.clear();
    }
}

#[test]
fn official_v4_windows_preserve_samples_across_fifteen_second_cuts_and_partial_tail() {
    for samples in [240_173, 480_173] {
        let origin = 13_579;
        let input: Vec<_> = (0..samples).map(|i| (i + 1) as f32 / 600_000.0).collect();
        let mut detector = OfficialV4Window {
            calls: 0,
            last_frame: vec![],
        };
        let mut segmenter = Segmenter::new(Source::Microphone, origin);
        let mut utterances = vec![];
        let mut offset = 0;
        // Native 10ms packets and irregular producer chunks must feed the same
        // complete 512-point windows, independently of ASR's exact sample cap.
        for size in [160, 37, 1024, 83].into_iter().cycle() {
            if offset == samples {
                break;
            }
            let end = (offset + size).min(samples);
            utterances.extend(segmenter.push(&input[offset..end], &mut detector).unwrap());
            offset = end;
        }
        utterances.extend(segmenter.finish(&mut detector).unwrap());
        let mut expected_lengths = vec![240_000; samples / 240_000];
        expected_lengths.push(samples % 240_000);
        assert_eq!(
            utterances
                .iter()
                .map(|u| u.samples.len())
                .collect::<Vec<_>>(),
            expected_lengths
        );
        assert!(utterances.iter().all(|u| u.samples.len() <= 240_000));
        assert_eq!(
            utterances
                .iter()
                .flat_map(|u| u.samples.iter().copied())
                .collect::<Vec<_>>(),
            input
        );
        let mut position = origin;
        for utterance in &utterances {
            assert_eq!(utterance.start_qpc, position);
            position += utterance.samples.len() as u64 * 625;
            assert_eq!(utterance.end_qpc, position);
        }
        assert_eq!(position, origin + samples as u64 * 625);
        assert_eq!(detector.calls, samples.div_ceil(512));
        let tail = samples % 512;
        assert_eq!(&detector.last_frame[..tail], &input[samples - tail..]);
        assert!(detector.last_frame[tail..]
            .iter()
            .all(|&sample| sample == 0.0));
    }
}

struct V4Decisions {
    calls: usize,
    decisions: Vec<bool>,
}
impl Detection for V4Decisions {
    fn speech(&mut self, frame: &[f32; VAD_SAMPLES]) -> Result<bool, String> {
        assert_eq!(frame.len(), 512);
        let speech = self.decisions.get(self.calls).copied().unwrap_or(false);
        self.calls += 1;
        Ok(speech)
    }
    fn reset(&mut self) {
        self.calls = 0;
    }
}

#[test]
fn hangover_crossing_fifteen_second_cap_never_sends_a_silence_only_tail_to_asr() {
    let fixture = Fixture::new(1600, 0.0);
    let raw = fixture.raw();
    let mut writer = fixture.store.edit("synthetic").unwrap();
    let cancelled = AtomicBool::new(false);
    let mut transcript = MeetingTranscript {
        id: "synthetic".into(),
        complete: false,
        segments: vec![],
        warnings: vec![],
    };
    let mut calls = vec![];
    let mut output = Output {
        writer: &mut writer,
        transcript: &mut transcript,
        cancelled: &cancelled,
        origin_qpc: BASE,
        transcribe: |audio: Vec<f32>| {
            calls.push((audio.len(), audio.iter().all(|sample| *sample == 0.0)));
            Ok("text".into())
        },
    };
    let mut detector = V4Decisions {
        calls: 0,
        decisions: (0..478).map(|i| i < 468).collect(),
    };
    let mut segmenter = Segmenter::new(Source::Remote, BASE);
    for i in 0..478 {
        let frame = if i < 468 { [0.2; 512] } else { [0.0; 512] };
        assert!(output
            .emit(segmenter.push(&frame, &mut detector).unwrap())
            .unwrap());
    }
    assert!(output
        .emit(segmenter.finish(&mut detector).unwrap())
        .unwrap());
    drop(output);
    assert_eq!(calls, [(240_000, false)]);
    assert_eq!(detector.calls, 478);
    assert_eq!(raw, fixture.raw());
}

#[test]
fn speech_after_cap_crossing_false_frame_keeps_every_weak_edge_sample_once() {
    let base = BASE + 625;
    let frames = 480;
    let input: Vec<f32> = (0..frames * 512).map(|i| (i + 1) as f32).collect();
    let mut detector = V4Decisions {
        calls: 0,
        decisions: (0..frames).map(|i| i < 468 || i == 469).collect(),
    };
    let mut segmenter = Segmenter::new(Source::Microphone, base);
    let mut utterances = vec![];
    for packet in input.chunks(160) {
        utterances.extend(segmenter.push(packet, &mut detector).unwrap());
    }
    utterances.extend(segmenter.finish(&mut detector).unwrap());
    assert_eq!(
        utterances
            .iter()
            .map(|u| u.samples.len())
            .collect::<Vec<_>>(),
        [240_000, 5760]
    );
    assert_eq!(
        utterances
            .iter()
            .flat_map(|u| u.samples.iter().copied())
            .collect::<Vec<_>>(),
        input
    );
    assert_eq!(utterances[0].end_qpc, utterances[1].start_qpc);
    assert_eq!(utterances[1].start_qpc, base + 240_000 * 625);
    assert_eq!(utterances[1].end_qpc, base + input.len() as u64 * 625);
    assert_eq!(detector.calls, frames);
}

// Removing pre-roll, hangover or the speech requirement loses these weak edges.
#[test]
fn real_segmenter_retains_weak_edges_but_never_emits_silence() {
    let mut detector = Level;
    let mut s = Segmenter::new(Source::Microphone, 10_000_000);
    let mut output = vec![];
    for amplitude in [0.002, 0.003, 0.2, 0.004, 0.005] {
        output.extend(s.push(&[amplitude; 480], &mut detector).unwrap());
    }
    output.extend(s.finish(&mut detector).unwrap());
    assert_eq!(output.len(), 1);
    assert_eq!(output[0].samples.len(), 2400);
    assert_eq!(
        (output[0].start_qpc, output[0].end_qpc),
        (10_000_000, 11_500_000)
    );
    assert_eq!(output[0].samples[0], 0.002);
    assert_eq!(output[0].samples[2399], 0.005);
    let mut silence = Segmenter::new(Source::Remote, 0);
    for _ in 0..600 {
        assert!(silence.push(&[0.0; 480], &mut detector).unwrap().is_empty());
    }
    assert!(silence.finish(&mut detector).unwrap().is_empty());
}

#[test]
fn continuous_speech_is_bounded_to_fifteen_seconds_without_duplicates() {
    let mut s = Segmenter::new(Source::Remote, 0);
    let mut detector = Constant(true);
    let mut all = vec![];
    let input: Vec<f32> = (0..528_000)
        .map(|i| ((i * 17 % 1009) as f32) / 2018.0)
        .collect();
    for frame in input.chunks(160) {
        all.extend(s.push(frame, &mut detector).unwrap());
    }
    all.extend(s.finish(&mut detector).unwrap());
    assert_eq!(
        all.iter().map(|u| u.samples.len()).collect::<Vec<_>>(),
        [240_000, 240_000, 48_000]
    );
    assert_eq!(
        all.iter()
            .flat_map(|u| u.samples.iter().copied())
            .collect::<Vec<_>>(),
        input
    );
    assert_eq!(
        all.iter()
            .map(|u| (u.start_qpc, u.end_qpc))
            .collect::<Vec<_>>(),
        [
            (0, 150_000_000),
            (150_000_000, 300_000_000),
            (300_000_000, 330_000_000)
        ]
    );
}

#[test]
fn final_short_frame_is_classified_with_padding_but_asr_receives_only_real_samples() {
    let mut s = Segmenter::new(Source::Microphone, 7);
    let mut detector = Constant(true);
    assert!(s.push(&[0.2; 173], &mut detector).unwrap().is_empty());
    let output = s.finish(&mut detector).unwrap();
    assert_eq!(output[0].samples, [0.2; 173]);
    assert_eq!(output[0].end_qpc, 108_132);
}

static NEXT: AtomicU64 = AtomicU64::new(0);
const BASE: u64 = 1_000_000_000;
struct Fixture {
    root: PathBuf,
    store: MeetingStore,
}
impl Fixture {
    fn new(samples: usize, amplitude: f32) -> Self {
        Self::epochs(&[(0, samples)], amplitude)
    }
    fn epochs(epochs: &[(usize, usize)], amplitude: f32) -> Self {
        let root = std::env::current_dir()
            .unwrap()
            .join("outputs")
            .join(format!(
                "processing-synthetic-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        let store = MeetingStore::new(&root).unwrap();
        let mut writer = store
            .create("synthetic", "Synthetic", BASE, vec![])
            .unwrap();
        for source in [Source::Microphone, Source::Remote] {
            for (epoch, (start, samples)) in epochs.iter().copied().enumerate() {
                let mut offset = 0;
                let mut sequence = 0;
                while offset < samples {
                    let n = (samples - offset).min(480);
                    let data: Vec<u8> = (offset..offset + n)
                        .flat_map(|i| ((i as f32 * 0.13).sin() * amplitude).to_le_bytes())
                        .collect();
                    writer
                        .append(CapturePacket {
                            source,
                            epoch: epoch as u32,
                            sequence,
                            device_frame_position: offset as u64,
                            qpc_100ns: BASE + (start + offset) as u64 * 625,
                            frames: n as u32,
                            flags: 0,
                            format: AudioFormat {
                                sample_rate: 16000,
                                channels: 1,
                                bits_per_sample: 32,
                                valid_bits: 32,
                                channel_mask: 0,
                                encoding: AudioEncoding::Float,
                            },
                            data,
                        })
                        .unwrap();
                    offset += n;
                    sequence += 1;
                }
                writer
                    .heartbeat(source, epoch as u32, BASE + (start + samples) as u64 * 625)
                    .unwrap();
            }
        }
        let end = epochs
            .iter()
            .map(|(start, count)| start + count)
            .max()
            .unwrap();
        writer
            .finish(MeetingStatus::Recorded, Some(BASE + end as u64 * 625), None)
            .unwrap();
        Self { root, store }
    }
    fn raw(&self) -> Vec<Vec<u8>> {
        let m = self.store.load("synthetic").unwrap();
        m.chunks
            .iter()
            .map(|c| fs::read(self.root.join("synthetic").join(&c.file)).unwrap())
            .collect()
    }
    fn transcript(&self) -> MeetingTranscript {
        serde_json::from_slice(&self.store.read_transcript("synthetic").unwrap().unwrap()).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn components(speech: bool) -> Result<(MeetingApm, Constant, Constant), String> {
    Ok((
        MeetingApm::new().map_err(str::to_owned)?,
        Constant(speech),
        Constant(speech),
    ))
}

#[test]
fn beta_attribution_warning_survives_setup_failure_cancellation_and_retry() {
    let fixture = Fixture::new(1600, 0.02);
    let raw = fixture.raw();
    let failed = process_with::<MeetingApm, Constant>(
        &fixture.store,
        "synthetic",
        &AtomicBool::new(false),
        |_| panic!("setup failure must not run ASR"),
        |_| {},
        || Err("meeting_vad_init".into()),
    );
    assert_eq!(failed.unwrap_err(), "meeting_vad_init");
    assert!(fixture
        .transcript()
        .warnings
        .iter()
        .any(|w| w == "meeting_attribution_experimental"));
    let cancelled = process_with::<MeetingApm, Constant>(
        &fixture.store,
        "synthetic",
        &AtomicBool::new(true),
        |_| panic!("cancelled job must not run ASR"),
        |_| {},
        || panic!("cancelled job must not initialize models"),
    )
    .unwrap();
    assert!(!cancelled.complete);
    assert!(cancelled
        .warnings
        .iter()
        .any(|w| w == "meeting_attribution_experimental"));
    let retried = process_with(
        &fixture.store,
        "synthetic",
        &AtomicBool::new(false),
        |_| Ok("synthetic text".into()),
        |_| {},
        || components(true),
    )
    .unwrap();
    assert!(retried.complete);
    assert!(retried
        .warnings
        .iter()
        .any(|w| w == "meeting_attribution_experimental"));
    assert_eq!(fixture.raw(), raw);
}

#[test]
fn pipeline_keeps_simultaneous_tracks_common_times_and_exact_tail_and_progress() {
    let fixture = Fixture::new(16_173, 0.02);
    let raw = fixture.raw();
    let mut lengths = vec![];
    let mut progress = vec![];
    let transcript = process_with(
        &fixture.store,
        "synthetic",
        &AtomicBool::new(false),
        |x| {
            lengths.push(x.len());
            Ok("text".into())
        },
        |p| progress.push(p),
        || components(true),
    )
    .unwrap();
    assert!(transcript.complete);
    assert_eq!(lengths, [16_173, 16_173]);
    assert_eq!(transcript.segments.len(), 2);
    for seg in &transcript.segments {
        assert_eq!((seg.start_ms, seg.end_ms), (0, 1011));
    }
    assert_eq!(
        fixture.store.load("synthetic").unwrap().status,
        MeetingStatus::Complete
    );
    assert_eq!(fixture.raw(), raw);
    assert_eq!(progress.first(), Some(&0));
    assert_eq!(progress.last(), Some(&100));
    assert!(progress.windows(2).all(|p| p[0] < p[1]));
}

#[test]
fn no_speech_never_calls_asr_and_retry_replaces_previous_transcript() {
    let fixture = Fixture::new(3200, 0.0);
    let first = process_with(
        &fixture.store,
        "synthetic",
        &AtomicBool::new(false),
        |_| Ok("old".into()),
        |_| {},
        || components(true),
    )
    .unwrap();
    assert_eq!(first.segments.len(), 2);
    let second = process_with(
        &fixture.store,
        "synthetic",
        &AtomicBool::new(false),
        |_| panic!("silence reached ASR"),
        |_| {},
        || components(false),
    )
    .unwrap();
    assert!(second.complete);
    assert!(second.segments.is_empty());
    assert!(fixture.transcript().segments.is_empty());
}

#[test]
fn asr_failure_preserves_completed_partial_and_raw_and_sanitizes_error() {
    let fixture = Fixture::new(248_000, 0.02);
    let raw = fixture.raw();
    let mut calls = 0;
    let result = process_with(
        &fixture.store,
        "synthetic",
        &AtomicBool::new(false),
        |_| {
            calls += 1;
            if calls == 1 {
                Ok("kept".into())
            } else {
                Err("private model path or content".into())
            }
        },
        |_| {},
        || components(true),
    );
    assert_eq!(result.unwrap_err(), "meeting_transcription_failed");
    let partial = fixture.transcript();
    assert!(!partial.complete);
    assert_eq!(partial.segments.len(), 1);
    assert_eq!(partial.segments[0].text, "kept");
    let manifest = fixture.store.load("synthetic").unwrap();
    assert_eq!(manifest.status, MeetingStatus::Failed);
    assert_eq!(
        manifest.error.as_deref(),
        Some("meeting_transcription_failed")
    );
    assert_eq!(raw, fixture.raw());
}

#[test]
fn cancellation_inside_asr_discards_its_new_text_and_retains_earlier_segments() {
    let fixture = Fixture::new(248_000, 0.02);
    let raw = fixture.raw();
    let cancelled = AtomicBool::new(false);
    let mut calls = 0;
    let partial = process_with(
        &fixture.store,
        "synthetic",
        &cancelled,
        |_| {
            calls += 1;
            if calls == 2 {
                cancelled.store(true, Ordering::Release);
            }
            Ok(format!("call{calls}"))
        },
        |_| {},
        || components(true),
    )
    .unwrap();
    assert!(!partial.complete);
    assert_eq!(partial.segments.len(), 1);
    assert_eq!(partial.segments[0].text, "call1");
    assert_eq!(
        fixture.store.load("synthetic").unwrap().status,
        MeetingStatus::Cancelled
    );
    assert_eq!(raw, fixture.raw());
}

struct BrokenEcho;
impl EchoProcessing for BrokenEcho {
    fn process(
        &mut self,
        _: &[f32; 160],
        _: &[f32; 160],
        _: &mut [f32; 160],
    ) -> Result<(), String> {
        Err("meeting_apm_process".into())
    }
    fn reset(&mut self) -> Result<(), String> {
        Ok(())
    }
}
#[test]
fn native_failure_stops_job_without_a_successful_empty_transcript() {
    let fixture = Fixture::new(3200, 0.02);
    let raw = fixture.raw();
    let result = process_with(
        &fixture.store,
        "synthetic",
        &AtomicBool::new(false),
        |_| panic!("failed APM reached ASR"),
        |_| {},
        || Ok((BrokenEcho, Constant(true), Constant(true))),
    );
    assert_eq!(result.unwrap_err(), "meeting_apm_process");
    assert!(!fixture.transcript().complete);
    assert_eq!(
        fixture.store.load("synthetic").unwrap().status,
        MeetingStatus::Failed
    );
    assert_eq!(raw, fixture.raw());
}

#[test]
fn normalized_derived_apm_inputs_clamp_once_without_modifying_float_raw() {
    let fixture = Fixture::new(1600, 1.01);
    let raw = fixture.raw();
    let result = process_with(
        &fixture.store,
        "synthetic",
        &AtomicBool::new(false),
        |_| Ok("text".into()),
        |_| {},
        || components(true),
    )
    .unwrap();
    assert!(result.complete);
    assert_eq!(
        result
            .warnings
            .iter()
            .filter(|w| w.as_str() == "meeting_apm_input_clamped")
            .count(),
        1
    );
    assert_eq!(raw, fixture.raw());
}

#[test]
fn epoch_restart_flushes_tail_resets_adaptive_state_and_retains_pause_time() {
    let fixture = Fixture::epochs(&[(0, 3200), (6400, 3200)], 0.02);
    let mut audio = vec![];
    let result = process_with(
        &fixture.store,
        "synthetic",
        &AtomicBool::new(false),
        |x| {
            audio.push(x);
            Ok("same".into())
        },
        |_| {},
        || components(true),
    )
    .unwrap();
    assert_eq!(result.segments.len(), 4);
    assert_eq!(audio.len(), 4);
    assert_eq!(audio[0], audio[2]);
    assert_eq!(audio[1], audio[3]);
    assert!(audio.iter().all(|x| x.len() == 3200));
    assert_eq!(
        result
            .segments
            .iter()
            .map(|s| (s.start_ms, s.end_ms))
            .collect::<Vec<_>>(),
        [(0, 200), (0, 200), (400, 600), (400, 600)]
    );
}

#[test]
fn cancellation_before_setup_publishes_cancelled_and_releases_exclusive_lease() {
    let fixture = Fixture::new(1600, 0.0);
    let raw = fixture.raw();
    let result = process_with::<MeetingApm, Constant>(
        &fixture.store,
        "synthetic",
        &AtomicBool::new(true),
        |_| panic!("cancelled ASR"),
        |_| {
            assert!(matches!(
                fixture.store.edit("synthetic"),
                Err("meeting_active")
            ));
            assert_eq!(fixture.store.delete("synthetic"), Err("meeting_active"));
        },
        || panic!("cancelled setup"),
    )
    .unwrap();
    assert!(!result.complete);
    assert!(result.segments.is_empty());
    assert_eq!(raw, fixture.raw());
    assert_eq!(
        fixture.store.load("synthetic").unwrap().status,
        MeetingStatus::Cancelled
    );
    assert!(fixture.store.edit("synthetic").is_ok());
}

#[test]
fn public_entry_point_reports_missing_vad_as_static_failed_and_preserves_raw() {
    let fixture = Fixture::new(1600, 0.0);
    let raw = fixture.raw();
    let result = process_meeting(
        &fixture.store,
        "synthetic",
        &fixture.root.join("nonexistent-synthetic-vad.onnx"),
        &AtomicBool::new(false),
        |_| panic!("missing VAD reached ASR"),
        |_| {},
    );
    assert_eq!(result.unwrap_err(), "meeting_vad_init");
    assert!(!fixture.transcript().complete);
    let manifest = fixture.store.load("synthetic").unwrap();
    assert_eq!(manifest.status, MeetingStatus::Failed);
    assert_eq!(manifest.error.as_deref(), Some("meeting_vad_init"));
    assert_eq!(raw, fixture.raw());
}

struct FailAfter {
    calls: usize,
}
impl Detection for FailAfter {
    fn speech(&mut self, _: &[f32; VAD_SAMPLES]) -> Result<bool, String> {
        self.calls += 1;
        if self.calls > MAX_SEGMENT_SAMPLES.div_ceil(VAD_SAMPLES) + 1 {
            Err("meeting_vad_process".into())
        } else {
            Ok(true)
        }
    }
    fn reset(&mut self) {
        self.calls = 0;
    }
}
#[test]
fn vad_failure_preserves_completed_partial_instead_of_marking_complete() {
    let fixture = Fixture::new(248_000, 0.02);
    let raw = fixture.raw();
    let result = process_with(
        &fixture.store,
        "synthetic",
        &AtomicBool::new(false),
        |_| Ok("kept".into()),
        |_| {},
        || {
            Ok((
                MeetingApm::new().map_err(str::to_owned)?,
                FailAfter { calls: 0 },
                FailAfter { calls: 0 },
            ))
        },
    );
    assert_eq!(result.unwrap_err(), "meeting_vad_process");
    let text = fixture.transcript();
    assert!(!text.complete);
    assert_eq!(text.segments.len(), 2);
    assert_eq!(
        fixture.store.load("synthetic").unwrap().status,
        MeetingStatus::Failed
    );
    assert_eq!(raw, fixture.raw());
}

fn interrupt(fixture: &Fixture) {
    let writer = fixture.store.edit("synthetic").unwrap();
    let end = writer.manifest().end_qpc_100ns;
    writer
        .finish(
            MeetingStatus::Interrupted,
            end,
            Some("capture_synthetic_failure".into()),
        )
        .unwrap();
}

#[test]
fn interrupted_capture_warning_survives_success_and_repeated_transcription() {
    let fixture = Fixture::new(1600, 0.02);
    let raw = fixture.raw();
    interrupt(&fixture);
    for _ in 0..2 {
        let transcript = process_with(
            &fixture.store,
            "synthetic",
            &AtomicBool::new(false),
            |_| Ok("text".into()),
            |_| {},
            || components(true),
        )
        .unwrap();
        assert!(transcript.complete);
        assert!(transcript
            .warnings
            .iter()
            .any(|w| w == "meeting_alignment_incomplete_recording"));
        let manifest = fixture.store.load("synthetic").unwrap();
        assert_eq!(manifest.status, MeetingStatus::Complete);
        assert_eq!(
            manifest.capture_error.as_deref(),
            Some("capture_synthetic_failure")
        );
        assert_eq!(raw, fixture.raw());
    }
}

#[test]
fn interrupted_capture_warning_is_saved_before_setup_failure_and_survives_retry() {
    let fixture = Fixture::new(1600, 0.02);
    let raw = fixture.raw();
    interrupt(&fixture);
    let failed = process_with::<MeetingApm, SileroDetection>(
        &fixture.store,
        "synthetic",
        &AtomicBool::new(false),
        |_| panic!("failed VAD reached ASR"),
        |_| {},
        || {
            assert!(fixture
                .transcript()
                .warnings
                .iter()
                .any(|w| w == "meeting_alignment_incomplete_recording"));
            let first = SileroDetection::new(&fixture.root.join("missing-synthetic-vad.onnx"))?;
            Ok((
                MeetingApm::new().map_err(str::to_owned)?,
                first,
                SileroDetection::new(&fixture.root.join("missing-synthetic-vad.onnx"))?,
            ))
        },
    );
    assert_eq!(failed.unwrap_err(), "meeting_vad_init");
    assert_eq!(
        fixture.store.load("synthetic").unwrap().status,
        MeetingStatus::Failed
    );
    assert!(fixture
        .transcript()
        .warnings
        .iter()
        .any(|w| w == "meeting_alignment_incomplete_recording"));
    let retried = process_with(
        &fixture.store,
        "synthetic",
        &AtomicBool::new(false),
        |_| Ok("retry".into()),
        |_| {},
        || components(true),
    )
    .unwrap();
    assert!(retried.complete);
    assert!(retried
        .warnings
        .iter()
        .any(|w| w == "meeting_alignment_incomplete_recording"));
    assert_eq!(
        fixture
            .store
            .load("synthetic")
            .unwrap()
            .capture_error
            .as_deref(),
        Some("capture_synthetic_failure")
    );
    assert_eq!(raw, fixture.raw());
}

#[test]
fn interrupted_empty_capture_keeps_warning_when_alignment_initialization_fails() {
    let fixture = Fixture::new(0, 0.0);
    interrupt(&fixture);
    let result = process_with::<MeetingApm, Constant>(
        &fixture.store,
        "synthetic",
        &AtomicBool::new(false),
        |_| panic!("missing audio reached ASR"),
        |_| {},
        || panic!("alignment failure reached VAD"),
    );
    assert_eq!(
        result.unwrap_err(),
        "meeting_alignment_missing_microphone_audio"
    );
    let transcript = fixture.transcript();
    assert!(!transcript.complete);
    assert!(transcript
        .warnings
        .iter()
        .any(|w| w == "meeting_alignment_incomplete_recording"));
}

#[test]
fn fractional_origin_uses_one_ms_endpoint_without_overlapping_or_losing_source_audio() {
    for (offset, start_ms, boundary_ms) in [(1, 0, 15000), (8, 1, 15001), (9, 1, 15001)] {
        let fixture = Fixture::epochs(&[(offset, 256_173)], 0.02);
        let mut expected_remote = vec![];
        let mut aligned = AlignedMeeting::open(fixture.store.clone(), "synthetic").unwrap();
        while let Some(frame) = aligned.next_frame().unwrap() {
            expected_remote.extend_from_slice(&frame.remote[..frame.valid_samples as usize]);
        }
        let mut audio = vec![];
        let result = process_with(
            &fixture.store,
            "synthetic",
            &AtomicBool::new(false),
            |x| {
                audio.push(x);
                Ok("text".into())
            },
            |_| {},
            || components(true),
        )
        .unwrap();
        for source in [Source::Microphone, Source::Remote] {
            let segments: Vec<_> = result
                .segments
                .iter()
                .filter(|s| s.source == source)
                .collect();
            assert_eq!(
                segments
                    .iter()
                    .map(|s| (s.start_ms, s.end_ms))
                    .collect::<Vec<_>>(),
                [(start_ms, boundary_ms), (boundary_ms, 16011)]
            );
            assert_eq!(segments[0].end_ms, segments[1].start_ms);
            assert_eq!(
                segments
                    .iter()
                    .map(|s| audio[s.id as usize].len())
                    .collect::<Vec<_>>(),
                [240_000, 16_173]
            );
        }
        let actual_remote: Vec<_> = result
            .segments
            .iter()
            .filter(|s| s.source == Source::Remote)
            .flat_map(|s| audio[s.id as usize].iter().copied())
            .collect();
        assert_eq!(actual_remote, expected_remote);
    }
}

struct CountedV4Windows {
    source: usize,
    calls: usize,
    observations: Rc<RefCell<Vec<(usize, usize)>>>,
}
impl Detection for CountedV4Windows {
    fn speech(&mut self, frame: &[f32; VAD_SAMPLES]) -> Result<bool, String> {
        assert_eq!(frame.len(), 512);
        self.calls += 1;
        self.observations
            .borrow_mut()
            .push((self.source, self.calls));
        Ok(true)
    }
    fn reset(&mut self) {
        self.calls = 0;
    }
}

#[test]
fn native_packet_pipeline_keeps_independent_vad512_states_exact_cap_and_fractional_tail() {
    let samples = 240_173;
    let fixture = Fixture::epochs(&[(9, samples)], 0.02);
    let raw = fixture.raw();
    let mut expected_remote = vec![];
    let mut aligned = AlignedMeeting::open(fixture.store.clone(), "synthetic").unwrap();
    while let Some(frame) = aligned.next_frame().unwrap() {
        expected_remote.extend_from_slice(&frame.remote[..frame.valid_samples as usize]);
    }
    let observations = Rc::new(RefCell::new(vec![]));
    let mut audio = vec![];
    let transcript = process_with(
        &fixture.store,
        "synthetic",
        &AtomicBool::new(false),
        |samples| {
            audio.push(samples);
            Ok("text".into())
        },
        |_| {},
        || {
            Ok((
                MeetingApm::new().map_err(str::to_owned)?,
                CountedV4Windows {
                    source: 0,
                    calls: 0,
                    observations: observations.clone(),
                },
                CountedV4Windows {
                    source: 1,
                    calls: 0,
                    observations: observations.clone(),
                },
            ))
        },
    )
    .unwrap();
    assert!(transcript.complete);
    for (source_index, source) in [(0, Source::Microphone), (1, Source::Remote)] {
        let calls: Vec<_> = observations
            .borrow()
            .iter()
            .filter(|(id, _)| *id == source_index)
            .map(|(_, count)| *count)
            .collect();
        assert_eq!(calls, (1..=samples.div_ceil(512)).collect::<Vec<_>>());
        let segments: Vec<_> = transcript
            .segments
            .iter()
            .filter(|segment| segment.source == source)
            .collect();
        assert_eq!(
            segments
                .iter()
                .map(|segment| audio[segment.id as usize].len())
                .collect::<Vec<_>>(),
            [240_000, 173]
        );
        assert_eq!(
            segments
                .iter()
                .map(|segment| (segment.start_ms, segment.end_ms))
                .collect::<Vec<_>>(),
            [(1, 15001), (15001, 15011)]
        );
    }
    let actual_remote: Vec<_> = transcript
        .segments
        .iter()
        .filter(|segment| segment.source == Source::Remote)
        .flat_map(|segment| audio[segment.id as usize].iter().copied())
        .collect();
    assert_eq!(actual_remote, expected_remote);
    assert_eq!(actual_remote.len(), samples);
    assert_eq!(raw, fixture.raw());
}
