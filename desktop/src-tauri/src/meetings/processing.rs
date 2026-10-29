//! Bounded offline meeting transcription, independent of ordinary dictation.
//!
//! Audio is read only. The microphone uses complete AEC3 with its fixed sample
//! latency removed; remote audio keeps its original timeline. Two independent
//! Silero states select speech before ASR. Each call holds at most 15s of audio.
use super::{
    alignment::{AlignedFrame, AlignedMeeting},
    apm::{MeetingApm, LATENCY_SAMPLES},
    contracts::{MeetingSegment, MeetingTranscript},
    store::{MeetingStore, MeetingWriter},
    types::{MeetingStatus, Source},
};
use std::{
    collections::VecDeque,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

const RATE: usize = 16_000;
// Silero v4's supported 16 kHz windows start at 512 samples. The ONNX
// session itself does not enforce the public wrapper's minimum-length check.
const VAD_SAMPLES: usize = 512;
const MAX_SEGMENT_SAMPLES: usize = 15 * RATE;
// Ten complete 32 ms VAD windows retain at most 320 ms on either edge.
const PRE_ROLL_FRAMES: usize = 10;
const TAIL_FRAMES: usize = 10;
const SAMPLE_TICKS: u64 = 625;

trait Detection {
    fn speech(&mut self, frame: &[f32; VAD_SAMPLES]) -> Result<bool, String>;
    fn reset(&mut self);
}
struct SileroDetection(vad_rs::Vad);
impl SileroDetection {
    fn new(path: &Path) -> Result<Self, String> {
        vad_rs::Vad::new(&path, RATE)
            .map(Self)
            .map_err(|_| "meeting_vad_init".into())
    }
}
impl Detection for SileroDetection {
    fn speech(&mut self, frame: &[f32; VAD_SAMPLES]) -> Result<bool, String> {
        self.0
            .compute(frame)
            .map(|result| result.prob > 0.3)
            .map_err(|_| "meeting_vad_process".into())
    }
    fn reset(&mut self) {
        self.0.reset();
    }
}
trait EchoProcessing {
    fn process(
        &mut self,
        capture: &[f32; 160],
        render: &[f32; 160],
        out: &mut [f32; 160],
    ) -> Result<(), String>;
    fn reset(&mut self) -> Result<(), String>;
}
impl EchoProcessing for MeetingApm {
    fn process(
        &mut self,
        a: &[f32; 160],
        b: &[f32; 160],
        c: &mut [f32; 160],
    ) -> Result<(), String> {
        MeetingApm::process(self, a, b, c).map_err(str::to_owned)
    }
    fn reset(&mut self) -> Result<(), String> {
        MeetingApm::reset(self).map_err(str::to_owned)
    }
}

struct Utterance {
    source: Source,
    start_qpc: u64,
    end_qpc: u64,
    samples: Vec<f32>,
}
struct Segmenter {
    source: Source,
    base_qpc: u64,
    received: u64,
    frame: [f32; VAD_SAMPLES],
    filled: usize,
    pre_roll: VecDeque<(u64, Vec<f32>)>,
    active: Option<Utterance>,
    silent_frames: usize,
}
impl Segmenter {
    fn new(source: Source, base_qpc: u64) -> Self {
        Self {
            source,
            base_qpc,
            received: 0,
            frame: [0.0; VAD_SAMPLES],
            filled: 0,
            pre_roll: VecDeque::new(),
            active: None,
            silent_frames: 0,
        }
    }
    fn push(
        &mut self,
        mut input: &[f32],
        detector: &mut impl Detection,
    ) -> Result<Vec<Utterance>, String> {
        let mut emitted = vec![];
        while !input.is_empty() {
            let count = input.len().min(VAD_SAMPLES - self.filled);
            self.frame[self.filled..self.filled + count].copy_from_slice(&input[..count]);
            self.filled += count;
            self.received += count as u64;
            input = &input[count..];
            if self.filled == VAD_SAMPLES {
                self.classify(detector, &mut emitted)?;
            }
        }
        Ok(emitted)
    }
    fn classify(
        &mut self,
        detector: &mut impl Detection,
        emitted: &mut Vec<Utterance>,
    ) -> Result<(), String> {
        let count = self.filled;
        // Padding is only for VAD: ASR and times retain the actual samples.
        self.frame[count..].fill(0.0);
        let speech = detector.speech(&self.frame)?;
        let start = self.base_qpc + (self.received - count as u64) * SAMPLE_TICKS;
        let end = start + count as u64 * SAMPLE_TICKS;
        if speech && self.active.is_none() {
            let mut samples = Vec::with_capacity(MAX_SEGMENT_SAMPLES);
            let first = self.pre_roll.front().map_or(start, |(qpc, _)| *qpc);
            for (_, frame) in self.pre_roll.drain(..) {
                samples.extend(frame);
            }
            self.active = Some(Utterance {
                source: self.source,
                start_qpc: first,
                end_qpc: end,
                samples,
            });
        }
        if self.active.is_some() {
            self.silent_frames = if speech { 0 } else { self.silent_frames + 1 };
            let mut offset = 0;
            while offset < count {
                if self.active.is_none() && !speech {
                    // A cap can split a hangover window. Keep its weak edge
                    // for the next speech onset, but never submit a new ASR
                    // utterance containing only frames classified as silent.
                    self.pre_roll.push_back((
                        start + offset as u64 * SAMPLE_TICKS,
                        self.frame[offset..count].to_vec(),
                    ));
                    break;
                }
                // A legal 512-point VAD window can straddle the exact 15s
                // ASR cap. Continue speech immediately; a silent remainder
                // above joins pre-roll instead. Both retain the exact boundary.
                let active = self.active.get_or_insert_with(|| Utterance {
                    source: self.source,
                    start_qpc: start + offset as u64 * SAMPLE_TICKS,
                    end_qpc: start + offset as u64 * SAMPLE_TICKS,
                    samples: Vec::with_capacity(MAX_SEGMENT_SAMPLES),
                });
                let take = (count - offset).min(MAX_SEGMENT_SAMPLES - active.samples.len());
                active
                    .samples
                    .extend_from_slice(&self.frame[offset..offset + take]);
                offset += take;
                active.end_qpc = start + offset as u64 * SAMPLE_TICKS;
                if active.samples.len() == MAX_SEGMENT_SAMPLES {
                    emitted.push(self.active.take().unwrap());
                }
            }
            if self.silent_frames >= TAIL_FRAMES {
                if let Some(active) = self.active.take() {
                    emitted.push(active);
                }
                self.silent_frames = 0;
            }
            if self.active.is_none() {
                self.silent_frames = 0;
            }
        } else {
            self.pre_roll
                .push_back((start, self.frame[..count].to_vec()));
            if self.pre_roll.len() > PRE_ROLL_FRAMES {
                self.pre_roll.pop_front();
            }
        }
        self.filled = 0;
        Ok(())
    }
    fn finish(&mut self, detector: &mut impl Detection) -> Result<Vec<Utterance>, String> {
        let mut emitted = vec![];
        if self.filled > 0 {
            self.classify(detector, &mut emitted)?;
        }
        if let Some(active) = self.active.take() {
            emitted.push(active);
        }
        self.pre_roll.clear();
        Ok(emitted)
    }
}

/// Transcribe a stopped meeting under an exclusive store lease. Cancellation
/// returns incomplete text and persists Cancelled. Failures return static codes
/// and persist Failed with completed partial text. Retry replaces prior results.
pub fn process_meeting(
    store: &MeetingStore,
    id: &str,
    vad_model: &Path,
    cancelled: &AtomicBool,
    transcribe: impl FnMut(Vec<f32>) -> Result<String, String>,
    progress: impl FnMut(u8),
) -> Result<MeetingTranscript, String> {
    process_with(store, id, cancelled, transcribe, progress, || {
        Ok((
            MeetingApm::new().map_err(str::to_owned)?,
            SileroDetection::new(vad_model)?,
            SileroDetection::new(vad_model)?,
        ))
    })
}
fn persist(writer: &mut MeetingWriter, transcript: &MeetingTranscript) -> Result<(), String> {
    let bytes = serde_json::to_vec(transcript).map_err(|_| "meeting_transcript")?;
    writer.write_transcript(&bytes).map_err(str::to_owned)
}
fn warning(transcript: &mut MeetingTranscript, code: &str) {
    if !transcript.warnings.iter().any(|old| old == code) {
        transcript.warnings.push(code.into());
    }
}
// Both ends of every half-open segment use the same nearest-ms mapping. A
// shared sample boundary must remain a shared display boundary, including when
// the epoch starts between whole milliseconds on the meeting's QPC timeline.
fn endpoint_ms(qpc: u64, origin_qpc: u64) -> u64 {
    let ticks = qpc.saturating_sub(origin_qpc);
    ticks / 10_000 + u64::from(ticks % 10_000 >= 5_000)
}
struct Output<'a, T> {
    writer: &'a mut MeetingWriter,
    transcript: &'a mut MeetingTranscript,
    cancelled: &'a AtomicBool,
    origin_qpc: u64,
    transcribe: T,
}
impl<T: FnMut(Vec<f32>) -> Result<String, String>> Output<'_, T> {
    fn emit(&mut self, utterances: Vec<Utterance>) -> Result<bool, String> {
        for utterance in utterances {
            if self.cancelled.load(Ordering::Acquire) {
                return Ok(false);
            }
            let result = (self.transcribe)(utterance.samples);
            // Cancellation during ASR wins, including over a returned error.
            if self.cancelled.load(Ordering::Acquire) {
                return Ok(false);
            }
            let text = result.map_err(|_| "meeting_transcription_failed")?;
            if text.trim().is_empty() {
                continue;
            }
            let id =
                u32::try_from(self.transcript.segments.len()).map_err(|_| "meeting_transcript")?;
            self.transcript.segments.push(MeetingSegment {
                id,
                source: utterance.source,
                start_ms: endpoint_ms(utterance.start_qpc, self.origin_qpc),
                end_ms: endpoint_ms(utterance.end_qpc, self.origin_qpc),
                text,
            });
            self.transcript.segments.sort_by_key(|s| {
                (
                    s.start_ms,
                    if s.source == Source::Microphone { 0 } else { 1 },
                    s.end_ms,
                    s.id,
                )
            });
            persist(self.writer, self.transcript)?;
        }
        Ok(true)
    }
}

struct EpochAudio {
    epoch: u32,
    base_qpc: u64,
    input_samples: usize,
    output_samples: usize,
    tracks: [Segmenter; 2],
}
impl EpochAudio {
    fn new(epoch: u32, base_qpc: u64) -> Self {
        Self {
            epoch,
            base_qpc,
            input_samples: 0,
            output_samples: 0,
            tracks: [
                Segmenter::new(Source::Microphone, base_qpc),
                Segmenter::new(Source::Remote, base_qpc),
            ],
        }
    }
    fn microphone<T: FnMut(Vec<f32>) -> Result<String, String>>(
        &mut self,
        cleaned: &[f32; 160],
        detector: &mut impl Detection,
        output: &mut Output<'_, T>,
    ) -> Result<bool, String> {
        let first = LATENCY_SAMPLES.saturating_sub(self.output_samples).min(160);
        let last = (LATENCY_SAMPLES + self.input_samples)
            .saturating_sub(self.output_samples)
            .min(160);
        self.output_samples += 160;
        if first < last {
            return output.emit(self.tracks[0].push(&cleaned[first..last], detector)?);
        }
        Ok(true)
    }
    fn push<T: FnMut(Vec<f32>) -> Result<String, String>>(
        &mut self,
        frame: AlignedFrame,
        echo: &mut impl EchoProcessing,
        detectors: &mut [impl Detection; 2],
        output: &mut Output<'_, T>,
    ) -> Result<bool, String> {
        if frame.start_qpc_100ns != self.base_qpc + self.input_samples as u64 * SAMPLE_TICKS
            || !(1..=160).contains(&frame.valid_samples)
        {
            return Err("meeting_processing_clock".into());
        }
        let mut capture = frame.microphone;
        let mut render = frame.remote;
        let mut clipped = false;
        // Only derived APM input is clamped. Float raw and remote ASR are intact.
        for sample in capture.iter_mut().chain(&mut render) {
            if !sample.is_finite() {
                return Err("meeting_apm_sample".into());
            }
            if sample.abs() > 1.0 {
                *sample = sample.clamp(-1.0, 1.0);
                clipped = true;
            }
        }
        if clipped {
            warning(output.transcript, "meeting_apm_input_clamped");
        }
        let mut cleaned = [0.0; 160];
        echo.process(&capture, &render, &mut cleaned)?;
        self.input_samples += frame.valid_samples as usize;
        if !self.microphone(&cleaned, &mut detectors[0], output)? {
            return Ok(false);
        }
        output.emit(self.tracks[1].push(
            &frame.remote[..frame.valid_samples as usize],
            &mut detectors[1],
        )?)
    }
    fn finish<T: FnMut(Vec<f32>) -> Result<String, String>>(
        &mut self,
        echo: &mut impl EchoProcessing,
        detectors: &mut [impl Detection; 2],
        output: &mut Output<'_, T>,
    ) -> Result<bool, String> {
        // One zero frame releases the fixed 128-sample delay. Crop to raw length.
        let mut cleaned = [0.0; 160];
        echo.process(&[0.0; 160], &[0.0; 160], &mut cleaned)?;
        if !self.microphone(&cleaned, &mut detectors[0], output)? {
            return Ok(false);
        }
        for (track, detector) in self.tracks.iter_mut().zip(detectors) {
            if !output.emit(track.finish(detector)?)? {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

fn process_with<E: EchoProcessing, D: Detection>(
    store: &MeetingStore,
    id: &str,
    cancelled: &AtomicBool,
    transcribe: impl FnMut(Vec<f32>) -> Result<String, String>,
    mut progress: impl FnMut(u8),
    components: impl FnOnce() -> Result<(E, D, D), String>,
) -> Result<MeetingTranscript, String> {
    let mut writer = store.edit(id).map_err(str::to_owned)?;
    let original = writer.manifest().clone();
    let end = original.end_qpc_100ns;
    writer
        .checkpoint(MeetingStatus::Processing, end, None)
        .map_err(str::to_owned)?;
    progress(0);
    let mut transcript = MeetingTranscript {
        id: id.into(),
        complete: false,
        segments: vec![],
        warnings: vec!["meeting_attribution_experimental".into()],
    };
    // Capture interruption is a recording fact, independent of this job's
    // mutable status. Include it in the very first partial publication so an
    // alignment/VAD setup failure or cancellation cannot erase that evidence.
    if original.capture_error.is_some()
        || original.capture_complete == Some(false)
        || (original.capture_complete.is_none() && original.status == MeetingStatus::Interrupted)
    {
        warning(&mut transcript, "meeting_alignment_incomplete_recording");
    }
    let result = (|| -> Result<bool, String> {
        persist(&mut writer, &transcript)?;
        if cancelled.load(Ordering::Acquire) {
            return Ok(false);
        }
        let mut aligned = AlignedMeeting::open(store.clone(), id)?;
        for observation in aligned.warnings() {
            warning(&mut transcript, observation.code);
        }
        let (mut echo, microphone, remote) = components()?;
        let mut detectors = [microphone, remote];
        let mut output = Output {
            writer: &mut writer,
            transcript: &mut transcript,
            cancelled,
            origin_qpc: original.start_qpc_100ns,
            transcribe,
        };
        let mut epoch: Option<EpochAudio> = None;
        let mut last_progress = 0;
        let duration = end
            .unwrap_or_else(|| {
                original
                    .chunks
                    .iter()
                    .map(|c| c.end_qpc_100ns)
                    .max()
                    .unwrap_or(original.start_qpc_100ns)
            })
            .saturating_sub(original.start_qpc_100ns)
            .max(1);
        loop {
            if cancelled.load(Ordering::Acquire) {
                return Ok(false);
            }
            let Some(frame) = aligned.next_frame()? else {
                break;
            };
            let frame_end = frame.start_qpc_100ns + frame.valid_samples as u64 * SAMPLE_TICKS;
            if epoch
                .as_ref()
                .is_some_and(|current| current.epoch != frame.epoch)
            {
                if !epoch.as_mut().ok_or("meeting_processing_epoch")?.finish(
                    &mut echo,
                    &mut detectors,
                    &mut output,
                )? {
                    return Ok(false);
                }
                echo.reset()?;
                for detector in &mut detectors {
                    detector.reset();
                }
                epoch = None;
            }
            let current =
                epoch.get_or_insert_with(|| EpochAudio::new(frame.epoch, frame.start_qpc_100ns));
            if !current.push(frame, &mut echo, &mut detectors, &mut output)? {
                return Ok(false);
            }
            let percent = ((frame_end.saturating_sub(original.start_qpc_100ns) as u128 * 100
                / duration as u128)
                .min(99)) as u8;
            if percent > last_progress {
                progress(percent);
                last_progress = percent;
            }
        }
        if let Some(current) = &mut epoch {
            if !current.finish(&mut echo, &mut detectors, &mut output)? {
                return Ok(false);
            }
        }
        Ok(!cancelled.load(Ordering::Acquire))
    })();
    let (status, error) = match &result {
        Ok(true) => (MeetingStatus::Complete, None),
        Ok(false) => (MeetingStatus::Cancelled, None),
        Err(code) => (MeetingStatus::Failed, Some(code.clone())),
    };
    transcript.complete = matches!(result, Ok(true));
    // Retain the lease across both atomic publications; on publication failure,
    // best-effort rollback prevents partial text from retaining complete=true.
    let publication = persist(&mut writer, &transcript)
        .and_then(|_| writer.checkpoint(status, end, error).map_err(str::to_owned));
    if let Err(code) = publication {
        transcript.complete = false;
        let _ = persist(&mut writer, &transcript);
        let _ = writer.checkpoint(MeetingStatus::Failed, end, Some(code.clone()));
        return Err(code);
    }
    if transcript.complete {
        progress(100);
    }
    result.map(|_| transcript)
}

#[cfg(test)]
#[path = "processing_tests.rs"]
mod tests;
