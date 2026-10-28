//! Bounded offline device-clock alignment on the common 16 kHz QPC timeline.
//!
//! Two read-only validation passes retain chunk/clock-segment metadata, not audio;
//! rendering holds one raw packet and a FIR window per source. A contiguous device
//! clock uses fixed-window timestamp consensus and a 250us model-consistency gate.
//! Non-affine clocks, unavailable sources, and unknown discontinuities fail
//! explicitly rather than invent silence.
//! Only heartbeat-covered remote no-playback intervals can split a clock segment.
use super::{
    store::{ChunkReader, MeetingStore},
    types::*,
};
use rubato::{
    sinc_interpolator::{ScalarInterpolator, SincInterpolator},
    WindowFunction,
};
use std::collections::{BTreeMap, VecDeque};

const QPC_HZ: f64 = 10_000_000.0;
const SAMPLE_TICKS: u64 = 625;
const FRAME_TICKS: u64 = 100_000;
// Project engineering contracts, not a per-packet precision guarantee from WASAPI.
const CLOCK_MODEL_TOLERANCE: u64 = 2_500;
const MAX_BOUNDARY_PADDING: u64 = 2_500;
const MAX_INPUT_ANCHOR_UNCERTAINTY: f64 = 10_000.0;
const CLOCK_WINDOW_ANCHORS: usize = 32;
const MIN_CONSENSUS_ANCHORS: u64 = 64;
const GAP_THRESHOLD: f64 = 20_000.0;
const MIN_CLOCK_SPAN: f64 = 5_000_000.0;
// Metadata remains bounded even for pathological repeated remote stop/restart bursts.
const MAX_CLOCK_SEGMENTS: usize = 16_384;
const BASE_SINC_LEN: usize = 256;
const BASE_SINC_RATE: usize = 48_000;
const SINC_PHASES: usize = 256;

/// One 10 ms frame on the original monotonic QPC timeline.
#[derive(Debug)]
pub struct AlignedFrame {
    pub epoch: u32,
    pub start_qpc_100ns: u64,
    pub microphone: [f32; 160],
    pub remote: [f32; 160],
    /// Actual samples before the epoch ends; trailing array entries are zero padding.
    pub valid_samples: u16,
}

/// Explicit non-fatal boundary/health observation, without audio or device identifiers.
#[derive(Debug)]
pub struct AlignmentWarning {
    pub source: Source,
    pub epoch: u32,
    pub qpc_100ns: u64,
    pub code: &'static str,
}

/// Streaming alignment reader; original meeting files are never modified.
pub struct AlignedMeeting {
    tracks: [Track; 2],
    epochs: Vec<Epoch>,
    epoch_index: usize,
    frame_index: u64,
    warnings: Vec<AlignmentWarning>,
    failure: Option<String>,
}
impl AlignedMeeting {
    /// Validate stored capture clocks and open a bounded read-only alignment stream.
    pub fn open(store: MeetingStore, id: &str) -> Result<Self, String> {
        let manifest = store.load(id).map_err(str::to_owned)?;
        if matches!(
            manifest.status,
            MeetingStatus::Recording | MeetingStatus::Paused
        ) {
            return Err("meeting_alignment_active".into());
        }
        let mut warnings = vec![];
        let microphone = scan_source(&store, id, &manifest, Source::Microphone, &mut warnings)?;
        if microphone.is_empty() {
            return Err("meeting_alignment_missing_microphone_audio".into());
        }
        let remote = scan_source(&store, id, &manifest, Source::Remote, &mut warnings)?;
        let mut bounds = BTreeMap::<u32, (u64, u64)>::new();
        // A common interval requires actual microphone samples. Remote startup
        // or drain packets may extend outside it and remain in the original
        // recording, but cannot manufacture microphone audio at either boundary.
        for segment in &microphone {
            bounds
                .entry(segment.epoch)
                .and_modify(|range| {
                    range.0 = range.0.min(segment.start);
                    range.1 = range.1.max(segment.end);
                })
                .or_insert((segment.start, segment.end));
        }
        let mut epochs: Vec<Epoch> = vec![];
        let recording_end = manifest.end_qpc_100ns.unwrap_or(u64::MAX);
        if recording_end < manifest.start_qpc_100ns {
            return Err("meeting_alignment_recording_interval".into());
        }
        for (epoch, (raw_start, raw_end)) in bounds {
            let start = raw_start.max(manifest.start_qpc_100ns);
            let end = raw_end.min(recording_end);
            if start >= end {
                continue;
            }
            if end <= start || epochs.last().is_some_and(|previous| previous.end > start) {
                return Err("meeting_alignment_epoch_clock".into());
            }
            check_coverage(
                Source::Microphone,
                epoch,
                start,
                end,
                &microphone,
                &manifest.source_health,
                &mut warnings,
            )?;
            check_coverage(
                Source::Remote,
                epoch,
                start,
                end,
                &remote,
                &manifest.source_health,
                &mut warnings,
            )?;
            epochs.push(Epoch { epoch, start, end });
        }
        if epochs.is_empty() {
            return Err("meeting_alignment_missing_microphone_audio".into());
        }
        for (source, segments) in [(Source::Microphone, &microphone), (Source::Remote, &remote)] {
            for segment in segments {
                if let Some(epoch) = epochs.iter().find(|e| e.epoch == segment.epoch) {
                    if segment.start < epoch.start || segment.end > epoch.end {
                        let boundary = if segment.start < epoch.start {
                            segment.start
                        } else {
                            epoch.end
                        };
                        observation(
                            &mut warnings,
                            source,
                            segment.epoch,
                            boundary,
                            "meeting_alignment_boundary_trimmed",
                        );
                    }
                } else if source == Source::Remote
                    && segment.start < recording_end
                    && segment.end > manifest.start_qpc_100ns
                {
                    return Err("meeting_alignment_missing_microphone_audio".into());
                } else {
                    observation(
                        &mut warnings,
                        source,
                        segment.epoch,
                        segment.start,
                        "meeting_alignment_boundary_trimmed",
                    );
                }
            }
        }
        if let Some(first) = epochs.first() {
            if first.start > manifest.start_qpc_100ns {
                observation(
                    &mut warnings,
                    Source::Microphone,
                    first.epoch,
                    manifest.start_qpc_100ns,
                    "meeting_alignment_boundary_trimmed",
                );
            }
        }
        if let Some(last) = epochs.last() {
            if recording_end != u64::MAX && last.end < recording_end {
                observation(
                    &mut warnings,
                    Source::Microphone,
                    last.epoch,
                    last.end,
                    "meeting_alignment_boundary_trimmed",
                );
            }
        }
        if matches!(
            manifest.status,
            MeetingStatus::Interrupted | MeetingStatus::Failed | MeetingStatus::Cancelled
        ) {
            for epoch in &epochs {
                warnings.push(AlignmentWarning {
                    source: Source::Microphone,
                    epoch: epoch.epoch,
                    qpc_100ns: epoch.start,
                    code: "meeting_alignment_incomplete_recording",
                });
            }
        }
        Ok(Self {
            tracks: [
                Track::new(&store, id, &manifest, Source::Microphone, microphone),
                Track::new(&store, id, &manifest, Source::Remote, remote),
            ],
            epochs,
            epoch_index: 0,
            frame_index: 0,
            warnings,
            failure: None,
        })
    }
    /// Return a common-clock frame, jumping directly over pauses between epochs.
    /// An error is terminal for this reader; reopen the original meeting to retry.
    pub fn next_frame(&mut self) -> Result<Option<AlignedFrame>, String> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        let result = self.render_next_frame();
        if let Err(error) = &result {
            self.failure = Some(error.clone());
        }
        result
    }
    fn render_next_frame(&mut self) -> Result<Option<AlignedFrame>, String> {
        let Some(epoch) = self.epochs.get(self.epoch_index) else {
            return Ok(None);
        };
        let start = epoch
            .start
            .checked_add(
                self.frame_index
                    .checked_mul(FRAME_TICKS)
                    .ok_or("meeting_alignment_clock_overflow")?,
            )
            .ok_or("meeting_alignment_clock_overflow")?;
        let valid_samples = (epoch.end - start).div_ceil(SAMPLE_TICKS).min(160) as u16;
        let mut frame = AlignedFrame {
            epoch: epoch.epoch,
            start_qpc_100ns: start,
            microphone: [0.0; 160],
            remote: [0.0; 160],
            valid_samples,
        };
        for sample in 0..160 {
            let qpc = start
                .checked_add(sample as u64 * SAMPLE_TICKS)
                .ok_or("meeting_alignment_clock_overflow")?;
            // The final partial 10ms frame is zero padded, never extended to the next epoch.
            if qpc >= epoch.end {
                break;
            }
            frame.microphone[sample] = self.tracks[0].sample(epoch, qpc)?;
            frame.remote[sample] = self.tracks[1].sample(epoch, qpc)?;
        }
        self.frame_index += 1;
        if start.saturating_add(FRAME_TICKS) >= epoch.end {
            self.epoch_index += 1;
            self.frame_index = 0;
        }
        Ok(Some(frame))
    }
    /// Return explicit observations accumulated while validating the original capture.
    pub fn warnings(&self) -> &[AlignmentWarning] {
        &self.warnings
    }
}

fn decode_mono(packet: &CapturePacket, frame: usize) -> Result<f32, String> {
    let format = &packet.format;
    if frame >= packet.frames as usize
        || !(1..=32).contains(&format.channels)
        || !(8000..=384000).contains(&format.sample_rate)
        || ![8, 16, 24, 32].contains(&format.bits_per_sample)
        || !(1..=format.bits_per_sample).contains(&format.valid_bits)
        || (format.encoding == AudioEncoding::Float
            && (format.bits_per_sample != 32 || format.valid_bits != 32))
    {
        return Err("meeting_alignment_format".into());
    }
    if packet.flags & 2 != 0 {
        return Ok(0.0);
    }
    let bytes = format.bits_per_sample as usize / 8;
    let channels = format.channels as usize;
    let align = bytes * channels;
    if packet.data.len() != packet.frames as usize * align {
        return Err("meeting_alignment_payload".into());
    }
    let mut sum = 0.0f64;
    for channel in 0..channels {
        let offset = frame * align + channel * bytes;
        let sample = &packet.data[offset..offset + bytes];
        let value = if format.encoding == AudioEncoding::Float {
            let value = f32::from_le_bytes([sample[0], sample[1], sample[2], sample[3]]);
            if !value.is_finite() {
                return Err("meeting_alignment_nonfinite".into());
            }
            value as f64
        } else {
            let mut raw = 0u32;
            for (byte, value) in sample.iter().enumerate() {
                raw |= (*value as u32) << (byte * 8);
            }
            let shift = format.bits_per_sample - format.valid_bits;
            let signed = if bytes == 1 {
                (raw >> shift) as i64 - (1i64 << (format.valid_bits - 1))
            } else {
                let full = if bytes == 4 {
                    raw as i32 as i64
                } else if raw & (1 << (format.bits_per_sample - 1)) != 0 {
                    raw as i64 - (1i64 << format.bits_per_sample)
                } else {
                    raw as i64
                };
                full >> shift
            };
            signed as f64 / (1u64 << (format.valid_bits - 1)) as f64
        };
        sum += value;
    }
    let value = (sum / channels as f64) as f32;
    if !value.is_finite() {
        return Err("meeting_alignment_sample_range".into());
    }
    Ok(value)
}

#[derive(Clone)]
struct Epoch {
    epoch: u32,
    start: u64,
    end: u64,
}
#[derive(Clone, Debug)]
struct ClockMap {
    origin_qpc: u64,
    intercept: f64,
    ticks_per_frame: f64,
    nominal_rate: bool,
}
impl ClockMap {
    fn position(&self, qpc: u64) -> f64 {
        ((qpc as i128 - self.origin_qpc as i128) as f64 - self.intercept) / self.ticks_per_frame
    }
    fn qpc(&self, position: f64) -> Result<u64, String> {
        let offset = self.intercept + position * self.ticks_per_frame;
        if !offset.is_finite() || offset.abs() > i64::MAX as f64 {
            return Err("meeting_alignment_clock_overflow".into());
        }
        self.origin_qpc
            .checked_add_signed(offset.round() as i64)
            .ok_or_else(|| "meeting_alignment_clock_overflow".into())
    }
}
// Welford covariance keeps two-hour device positions and QPC offsets precise without
// storing per-packet anchors or subtracting large nearly equal sums.
#[derive(Default)]
struct ClockRegression {
    count: u64,
    mean_x: f64,
    mean_y: f64,
    xx: f64,
    xy: f64,
    max_x: f64,
}
impl ClockRegression {
    fn push(&mut self, x: f64, y: f64) {
        self.count += 1;
        self.max_x = self.max_x.max(x);
        let dx = x - self.mean_x;
        let dy = y - self.mean_y;
        self.mean_x += dx / self.count as f64;
        self.mean_y += dy / self.count as f64;
        self.xx += dx * (x - self.mean_x);
        self.xy += dx * (y - self.mean_y);
    }
    fn finish(&self, origin_qpc: u64, rate: u32) -> Result<ClockMap, String> {
        self.finish_with_span(origin_qpc, rate, self.max_x)
    }
    fn finish_with_span(
        &self,
        origin_qpc: u64,
        rate: u32,
        span_frames: f64,
    ) -> Result<ClockMap, String> {
        let nominal = QPC_HZ / rate as f64;
        let nominal_rate = span_frames * nominal < MIN_CLOCK_SPAN;
        let ticks_per_frame = if self.xx > 0.0 && !nominal_rate {
            self.xy / self.xx
        } else {
            nominal
        };
        // Less than 0.5s of anchors cannot establish ppm drift under 250us
        // acquisition jitter. Use the nominal rate explicitly rather than fitting
        // an implausible rate to two anchors that could conceal an unknown gap.
        let uncertainty = if span_frames > 0.0 {
            2.0 * CLOCK_MODEL_TOLERANCE as f64 / (span_frames * nominal)
        } else {
            0.0
        };
        if !ticks_per_frame.is_finite()
            || ticks_per_frame <= 0.0
            || (ticks_per_frame / nominal - 1.0).abs() > 0.001 + uncertainty
        {
            return Err("meeting_alignment_clock_rate".into());
        }
        Ok(ClockMap {
            origin_qpc,
            intercept: self.mean_y - ticks_per_frame * self.mean_x,
            ticks_per_frame,
            nominal_rate,
        })
    }
}

#[derive(Clone, Copy)]
struct ClockAnchor {
    position: f64,
    time: f64,
    end_position: f64,
}

#[derive(Default)]
struct ClockWindow {
    anchors: Vec<ClockAnchor>,
    count: u64,
}

struct LocalClockFit {
    origin_position: f64,
    origin_time: f64,
    intercept: f64,
    slope: f64,
    first_position: f64,
    last_position: f64,
    end_position: f64,
    noisy_timestamps: bool,
    #[cfg(test)]
    pair_slopes: usize,
}

impl LocalClockFit {
    fn time(&self, position: f64) -> f64 {
        self.origin_time + self.intercept + self.slope * (position - self.origin_position)
    }
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_unstable_by(f64::total_cmp);
    let middle = values.len() / 2;
    if values.len() % 2 == 0 {
        (values[middle - 1] + values[middle]) / 2.0
    } else {
        values[middle]
    }
}

impl ClockWindow {
    fn push(&mut self, position: f64, time: f64, frames: u32) -> Result<(), String> {
        let anchor = ClockAnchor {
            position,
            time,
            end_position: position + f64::from(frames),
        };
        if self.anchors.len() < CLOCK_WINDOW_ANCHORS {
            self.anchors.push(anchor);
        } else {
            self.anchors[self.count as usize % CLOCK_WINDOW_ANCHORS] = anchor;
        }
        self.count = self
            .count
            .checked_add(1)
            .ok_or("meeting_alignment_clock_overflow")?;
        Ok(())
    }

    fn complete_batch(&self) -> bool {
        self.count != 0 && self.count % CLOCK_WINDOW_ANCHORS as u64 == 0
    }

    fn needs_tail_check(&self) -> bool {
        self.count >= CLOCK_WINDOW_ANCHORS as u64 && !self.complete_batch()
    }

    fn fit(&self) -> Result<LocalClockFit, String> {
        if self.anchors.len() != CLOCK_WINDOW_ANCHORS {
            return Err("meeting_alignment_state".into());
        }
        let origin = self.anchors[0];
        let mut slopes = Vec::with_capacity(CLOCK_WINDOW_ANCHORS * (CLOCK_WINDOW_ANCHORS - 1) / 2);
        for (index, left) in self.anchors.iter().enumerate() {
            for right in &self.anchors[index + 1..] {
                let dx = right.position - left.position;
                if dx == 0.0 {
                    return Err("meeting_alignment_position_regression".into());
                }
                let slope = (right.time - left.time) / dx;
                if !slope.is_finite() {
                    return Err("meeting_alignment_qpc_jitter".into());
                }
                slopes.push(slope);
            }
        }
        let slope = median(&mut slopes);
        if !slope.is_finite() || slope <= 0.0 {
            return Err("meeting_alignment_qpc_jitter".into());
        }
        // Theil-Sen's joint intercept. Local coordinates avoid cancellation of
        // large absolute QPC readings; a ring need not be reordered or copied.
        let mut intercepts: Vec<f64> = self
            .anchors
            .iter()
            .map(|anchor| (anchor.time - origin.time) - slope * (anchor.position - origin.position))
            .collect();
        let fit = LocalClockFit {
            origin_position: origin.position,
            origin_time: origin.time,
            intercept: median(&mut intercepts),
            slope,
            first_position: self
                .anchors
                .iter()
                .map(|a| a.position)
                .fold(f64::INFINITY, f64::min),
            last_position: self
                .anchors
                .iter()
                .map(|a| a.position)
                .fold(f64::NEG_INFINITY, f64::max),
            end_position: self
                .anchors
                .iter()
                .map(|a| a.end_position)
                .fold(f64::NEG_INFINITY, f64::max),
            noisy_timestamps: false,
            #[cfg(test)]
            pair_slopes: slopes.len(),
        };
        let mut inliers = 0;
        for anchor in &self.anchors {
            let residual = (anchor.time - fit.time(anchor.position)).abs();
            if residual > MAX_INPUT_ANCHOR_UNCERTAINTY {
                return Err("meeting_alignment_qpc_jitter".into());
            }
            if residual <= CLOCK_MODEL_TOLERANCE as f64 {
                inliers += 1;
            }
        }
        if inliers * 4 < self.anchors.len() * 3 {
            return Err("meeting_alignment_qpc_jitter".into());
        }
        Ok(LocalClockFit {
            noisy_timestamps: inliers != self.anchors.len(),
            ..fit
        })
    }
}

#[derive(Default)]
struct ClockValidationStats {
    noisy_timestamps: bool,
    #[cfg(test)]
    max_buffered_anchors: usize,
    #[cfg(test)]
    max_pair_slopes: usize,
}

struct ClockValidator<'a> {
    segment: &'a Segment,
    window: ClockWindow,
    stats: ClockValidationStats,
}

impl<'a> ClockValidator<'a> {
    fn new(segment: &'a Segment) -> Self {
        Self {
            segment,
            window: ClockWindow::default(),
            stats: ClockValidationStats::default(),
        }
    }

    fn push(&mut self, packet: &CapturePacket) -> Result<(), String> {
        let position = packet
            .device_frame_position
            .checked_sub(self.segment.first_position)
            .ok_or("meeting_alignment_position_regression")? as f64;
        if !self.segment.consensus_clock {
            if self.segment.clock.qpc(position)?.abs_diff(packet.qpc_100ns) > CLOCK_MODEL_TOLERANCE
            {
                return Err("meeting_alignment_qpc_jitter".into());
            }
            return Ok(());
        }
        let time = (packet.qpc_100ns as i128 - self.segment.clock.origin_qpc as i128) as f64;
        self.window.push(position, time, packet.frames)?;
        #[cfg(test)]
        {
            self.stats.max_buffered_anchors = self
                .stats
                .max_buffered_anchors
                .max(self.window.anchors.len());
        }
        if self.window.complete_batch() {
            self.validate_window()?;
        }
        Ok(())
    }

    fn validate_window(&mut self) -> Result<(), String> {
        let local = self.window.fit()?;
        for position in [local.first_position, local.end_position] {
            let fitted =
                self.segment.clock.intercept + self.segment.clock.ticks_per_frame * position;
            if (local.time(position) - fitted).abs() > CLOCK_MODEL_TOLERANCE as f64 {
                return Err("meeting_alignment_qpc_jitter".into());
            }
        }
        self.stats.noisy_timestamps |= local.noisy_timestamps;
        #[cfg(test)]
        {
            self.stats.max_pair_slopes = self.stats.max_pair_slopes.max(local.pair_slopes);
        }
        Ok(())
    }

    fn finish(mut self) -> Result<ClockValidationStats, String> {
        if self.segment.consensus_clock && self.window.needs_tail_check() {
            self.validate_window()?;
        }
        Ok(self.stats)
    }
}

#[derive(Clone)]
struct Segment {
    epoch: u32,
    first_packet: u64,
    end_packet: u64,
    first_position: u64,
    frames: u64,
    format: AudioFormat,
    clock: ClockMap,
    start: u64,
    end: u64,
    consensus_clock: bool,
}
struct Builder {
    segment: Segment,
    regression: ClockRegression,
    consensus_regression: ClockRegression,
    window: ClockWindow,
    consensus_error: Option<String>,
}
impl Builder {
    fn new(packet: &CapturePacket, ordinal: u64) -> Self {
        Self {
            segment: Segment {
                epoch: packet.epoch,
                first_packet: ordinal,
                end_packet: ordinal,
                first_position: packet.device_frame_position,
                frames: 0,
                format: packet.format.clone(),
                clock: ClockMap {
                    origin_qpc: packet.qpc_100ns,
                    intercept: 0.0,
                    ticks_per_frame: QPC_HZ / packet.format.sample_rate as f64,
                    nominal_rate: true,
                },
                start: 0,
                end: 0,
                consensus_clock: false,
            },
            regression: ClockRegression::default(),
            consensus_regression: ClockRegression::default(),
            window: ClockWindow::default(),
            consensus_error: None,
        }
    }
    fn push(&mut self, packet: &CapturePacket, ordinal: u64) -> Result<(), String> {
        let x = packet
            .device_frame_position
            .checked_sub(self.segment.first_position)
            .ok_or("meeting_alignment_position_regression")?;
        let y = packet.qpc_100ns as i128 - self.segment.clock.origin_qpc as i128;
        self.regression.push(x as f64, y as f64);
        self.window.push(x as f64, y as f64, packet.frames)?;
        if self.window.complete_batch() {
            match self.window.fit() {
                Ok(local) => {
                    let middle = (local.first_position + local.last_position) / 2.0;
                    self.consensus_regression.push(middle, local.time(middle));
                }
                Err(error) => {
                    self.consensus_error.get_or_insert(error);
                }
            }
        }
        self.segment.frames = self
            .segment
            .frames
            .checked_add(packet.frames as u64)
            .ok_or("meeting_alignment_clock_overflow")?;
        self.segment.end_packet = ordinal + 1;
        Ok(())
    }
    fn finish(mut self) -> Result<Segment, String> {
        self.segment.consensus_clock = self.regression.count >= MIN_CONSENSUS_ANCHORS
            && self.regression.max_x * QPC_HZ / self.segment.format.sample_rate as f64
                >= MIN_CLOCK_SPAN;
        self.segment.clock = if self.segment.consensus_clock {
            if let Some(error) = self.consensus_error {
                return Err(error);
            }
            self.consensus_regression.finish_with_span(
                self.segment.clock.origin_qpc,
                self.segment.format.sample_rate,
                self.regression.max_x,
            )?
        } else {
            self.regression.finish(
                self.segment.clock.origin_qpc,
                self.segment.format.sample_rate,
            )?
        };
        self.segment.start = self.segment.clock.qpc(0.0)?;
        self.segment.end = self.segment.clock.qpc(self.segment.frames as f64)?;
        Ok(self.segment)
    }
}

fn push_segment(segments: &mut Vec<Segment>, builder: Builder) -> Result<(), String> {
    if segments.len() >= MAX_CLOCK_SEGMENTS {
        return Err("meeting_alignment_segment_limit".into());
    }
    segments.push(builder.finish()?);
    Ok(())
}

struct PacketStream {
    store: MeetingStore,
    id: String,
    chunks: Vec<ChunkMeta>,
    next_chunk: usize,
    reader: Option<ChunkReader>,
    ordinal: u64,
}
impl PacketStream {
    fn new(store: &MeetingStore, id: &str, manifest: &MeetingManifest, source: Source) -> Self {
        Self {
            store: store.clone(),
            id: id.into(),
            chunks: manifest
                .chunks
                .iter()
                .filter(|chunk| chunk.source == source)
                .cloned()
                .collect(),
            next_chunk: 0,
            reader: None,
            ordinal: 0,
        }
    }
    fn next(&mut self) -> Result<Option<CapturePacket>, String> {
        loop {
            if let Some(reader) = &mut self.reader {
                if let Some(packet) = reader.next_packet().map_err(str::to_owned)? {
                    self.ordinal += 1;
                    return Ok(Some(packet));
                }
                self.reader = None;
            }
            let Some(chunk) = self.chunks.get(self.next_chunk) else {
                return Ok(None);
            };
            self.reader = Some(
                self.store
                    .open_reader_for_chunk(&self.id, chunk)
                    .map_err(str::to_owned)?,
            );
            self.next_chunk += 1;
        }
    }
}
#[derive(Clone, Copy)]
struct Previous {
    epoch: u32,
    sequence: u64,
    position: u64,
    qpc: u64,
    frames: u32,
    sample_rate: u32,
}
fn health_covers(
    health: &[SourceHealth],
    source: Source,
    epoch: u32,
    start: u64,
    end: u64,
) -> bool {
    health.iter().any(|h| {
        h.source == source
            && h.epoch == epoch
            && h.first_heartbeat_qpc_100ns <= start
            && h.last_heartbeat_qpc_100ns >= end
    })
}
fn observation(
    warnings: &mut Vec<AlignmentWarning>,
    source: Source,
    epoch: u32,
    qpc: u64,
    code: &'static str,
) {
    warnings.push(AlignmentWarning {
        source,
        epoch,
        qpc_100ns: qpc,
        code,
    });
}
fn scan_source(
    store: &MeetingStore,
    id: &str,
    manifest: &MeetingManifest,
    source: Source,
    warnings: &mut Vec<AlignmentWarning>,
) -> Result<Vec<Segment>, String> {
    let mut stream = PacketStream::new(store, id, manifest, source);
    let mut segments = vec![];
    let mut builder: Option<Builder> = None;
    let mut previous: Option<Previous> = None;
    while let Some(packet) = stream.next()? {
        if packet.flags & 4 != 0 {
            return Err("meeting_alignment_timestamp_error".into());
        }
        if packet.flags & !7 != 0 {
            return Err("meeting_alignment_flags".into());
        }
        packet
            .device_frame_position
            .checked_add(packet.frames as u64)
            .ok_or("meeting_alignment_clock_overflow")?;
        let mut new_segment = previous.is_none_or(|p| p.epoch != packet.epoch);
        if let Some(prev) = previous {
            if packet.epoch < prev.epoch {
                return Err("meeting_alignment_epoch_regression".into());
            }
            if packet.epoch == prev.epoch {
                if packet.sequence
                    != prev
                        .sequence
                        .checked_add(1)
                        .ok_or("meeting_alignment_sequence")?
                {
                    return Err("meeting_alignment_sequence".into());
                }
                if packet.qpc_100ns <= prev.qpc {
                    return Err("meeting_alignment_qpc_regression".into());
                }
                let expected =
                    prev.qpc as f64 + prev.frames as f64 * QPC_HZ / prev.sample_rate as f64;
                let gap = packet.qpc_100ns as f64 - expected;
                let position_gap =
                    packet.device_frame_position != prev.position + prev.frames as u64;
                let format_changed = builder
                    .as_ref()
                    .is_some_and(|b| b.segment.format != packet.format);
                if position_gap || gap.abs() > GAP_THRESHOLD || format_changed {
                    let gap_start = expected.round() as u64;
                    if source != Source::Remote
                        || gap <= GAP_THRESHOLD
                        || !health_covers(
                            &manifest.source_health,
                            source,
                            packet.epoch,
                            gap_start,
                            packet.qpc_100ns,
                        )
                    {
                        return Err("meeting_alignment_unknown_gap".into());
                    }
                    new_segment = true;
                    observation(
                        warnings,
                        source,
                        packet.epoch,
                        gap_start,
                        "meeting_alignment_remote_no_playback",
                    );
                }
                if packet.flags & 1 != 0 && !new_segment {
                    return Err("meeting_alignment_discontinuity".into());
                }
            }
        }
        if new_segment {
            if let Some(old) = builder.take() {
                push_segment(&mut segments, old)?;
            }
            builder = Some(Builder::new(&packet, stream.ordinal - 1));
            if packet.flags & 1 != 0 {
                let code = if previous.is_some_and(|p| p.epoch == packet.epoch) {
                    "meeting_alignment_remote_resume_discontinuity"
                } else {
                    "meeting_alignment_initial_discontinuity"
                };
                observation(warnings, source, packet.epoch, packet.qpc_100ns, code);
            }
        }
        builder
            .as_mut()
            .ok_or("meeting_alignment_state")?
            .push(&packet, stream.ordinal - 1)?;
        previous = Some(Previous {
            epoch: packet.epoch,
            sequence: packet.sequence,
            position: packet.device_frame_position,
            qpc: packet.qpc_100ns,
            frames: packet.frames,
            sample_rate: packet.format.sample_rate,
        });
    }
    if let Some(old) = builder {
        push_segment(&mut segments, old)?;
    }
    for segment in &segments {
        if segment.end_packet - segment.first_packet == 1 {
            observation(
                warnings,
                source,
                segment.epoch,
                segment.start,
                "meeting_alignment_single_anchor_nominal_rate",
            );
        } else if segment.clock.nominal_rate {
            observation(
                warnings,
                source,
                segment.epoch,
                segment.start,
                "meeting_alignment_short_span_nominal_rate",
            );
        }
    }
    // Validate every measured anchor against the fit before exposing any output. No
    // packet metadata grows with recording duration, and no original file is changed.
    let mut stream = PacketStream::new(store, id, manifest, source);
    let mut index = 0;
    let mut validator = segments.first().map(ClockValidator::new);
    while let Some(packet) = stream.next()? {
        while segments
            .get(index)
            .is_some_and(|s| stream.ordinal > s.end_packet)
        {
            if validator
                .take()
                .ok_or("meeting_alignment_state")?
                .finish()?
                .noisy_timestamps
            {
                let segment = &segments[index];
                observation(
                    warnings,
                    source,
                    segment.epoch,
                    segment.start,
                    "meeting_alignment_clock_timestamp_jitter",
                );
            }
            index += 1;
            validator = segments.get(index).map(ClockValidator::new);
        }
        validator
            .as_mut()
            .ok_or("meeting_alignment_state")?
            .push(&packet)?;
    }
    if let Some(validator) = validator {
        if validator.finish()?.noisy_timestamps {
            let segment = &segments[index];
            observation(
                warnings,
                source,
                segment.epoch,
                segment.start,
                "meeting_alignment_clock_timestamp_jitter",
            );
        }
    }
    Ok(segments)
}
fn check_coverage(
    source: Source,
    epoch: u32,
    start: u64,
    end: u64,
    segments: &[Segment],
    health: &[SourceHealth],
    warnings: &mut Vec<AlignmentWarning>,
) -> Result<(), String> {
    if !segments
        .iter()
        .any(|s| s.epoch == epoch && s.end > start && s.start < end)
    {
        if source == Source::Remote && health_covers(health, source, epoch, start, end) {
            observation(
                warnings,
                source,
                epoch,
                start,
                "meeting_alignment_remote_no_playback",
            );
            return Ok(());
        }
        return Err(if source == Source::Microphone {
            "meeting_alignment_missing_microphone_audio"
        } else {
            "meeting_alignment_source_unavailable"
        }
        .into());
    }
    let mut cursor = start;
    for segment in segments
        .iter()
        .filter(|s| s.epoch == epoch && s.end > start && s.start < end)
    {
        let segment_start = segment.start.max(start);
        let segment_end = segment.end.min(end);
        if segment_start < cursor && cursor != start {
            return Err("meeting_alignment_overlap".into());
        }
        check_gap(source, epoch, cursor, segment_start, health, warnings)?;
        cursor = cursor.max(segment_end);
    }
    check_gap(source, epoch, cursor, end, health, warnings)
}
fn check_gap(
    source: Source,
    epoch: u32,
    start: u64,
    end: u64,
    health: &[SourceHealth],
    warnings: &mut Vec<AlignmentWarning>,
) -> Result<(), String> {
    if end <= start {
        return Ok(());
    }
    if source == Source::Remote && health_covers(health, source, epoch, start, end) {
        observation(
            warnings,
            source,
            epoch,
            start,
            "meeting_alignment_remote_no_playback",
        );
        return Ok(());
    }
    // Small fitted edge differences (<=250us) are explicit, traceable padding.
    // Larger unknown holes are never silently converted into healthy silence.
    if end - start <= MAX_BOUNDARY_PADDING {
        observation(
            warnings,
            source,
            epoch,
            start,
            "meeting_alignment_boundary_padding",
        );
        return Ok(());
    }
    Err("meeting_alignment_source_unavailable".into())
}

struct Track {
    stream: PacketStream,
    segments: Vec<Segment>,
    index: usize,
    window: Option<NativeWindow>,
}
impl Track {
    fn new(
        store: &MeetingStore,
        id: &str,
        manifest: &MeetingManifest,
        source: Source,
        segments: Vec<Segment>,
    ) -> Self {
        Self {
            stream: PacketStream::new(store, id, manifest, source),
            segments,
            index: 0,
            window: None,
        }
    }
    fn sample(&mut self, epoch: &Epoch, qpc: u64) -> Result<f32, String> {
        while self
            .segments
            .get(self.index)
            .is_some_and(|s| s.epoch < epoch.epoch || (s.epoch == epoch.epoch && s.end <= qpc))
        {
            self.index += 1;
            self.window = None;
        }
        let Some(segment) = self.segments.get(self.index) else {
            return Ok(0.0);
        };
        if segment.epoch != epoch.epoch || segment.start > qpc {
            return Ok(0.0);
        }
        if self.window.is_none() {
            while self.stream.ordinal < segment.first_packet {
                self.stream.next()?.ok_or("meeting_alignment_truncated")?;
            }
            self.window = Some(NativeWindow::new(segment, epoch));
        }
        self.window
            .as_mut()
            .ok_or("meeting_alignment_state")?
            .sample(segment, qpc, &mut self.stream)
    }
}
struct NativeWindow {
    filter: ScalarInterpolator<f32>,
    support: Vec<f32>,
    cache: VecDeque<f32>,
    cache_start: u64,
    frames_read: u64,
    packet: Option<CapturePacket>,
    packet_frame: usize,
    input_start: f64,
    input_end: f64,
}
impl NativeWindow {
    fn new(segment: &Segment, epoch: &Epoch) -> Self {
        let actual_rate = QPC_HZ / segment.clock.ticks_per_frame;
        let cutoff = (0.925 * (16000.0 / actual_rate).min(1.0)) as f32;
        // Keep at least the 256/48k-second aperture across native rates. FIR
        // transition width in Hz scales as native_rate / tap_count; increasing
        // taps with rate preserves that physical transition band. rubato requires
        // a multiple of eight. The validated 384k ceiling bounds this at 2048
        // taps (2050 support samples), independent of recording duration.
        let sinc_len =
            (BASE_SINC_LEN * segment.format.sample_rate as usize).div_ceil(BASE_SINC_RATE * 8) * 8;
        let sinc_len = sinc_len.max(BASE_SINC_LEN);
        Self {
            filter: ScalarInterpolator::new(
                sinc_len,
                SINC_PHASES,
                cutoff,
                WindowFunction::BlackmanHarris2,
            ),
            support: vec![0.0; sinc_len + 2],
            cache: VecDeque::with_capacity(sinc_len + 2),
            cache_start: 0,
            frames_read: 0,
            packet: None,
            packet_frame: 0,
            input_start: segment.clock.position(epoch.start),
            input_end: segment.clock.position(epoch.end),
        }
    }
    fn sample(
        &mut self,
        segment: &Segment,
        qpc: u64,
        stream: &mut PacketStream,
    ) -> Result<f32, String> {
        let position = segment.clock.position(qpc);
        let sinc_len = self.filter.len();
        // rubato's public sinc bank has its center at L/2-1+(phase+1)/N.
        // Compensating that published implementation's bank convention keeps the
        // requested physical sample position exact, including fractional starts.
        // FixedOut exposes rate/delay but no fractional starting phase, so its
        // public interpolation kernel is used directly on this bounded window.
        let time = position - sinc_len as f64 / 2.0 + 1.0 - 1.0 / SINC_PHASES as f64;
        let first = time.floor() as i64;
        let phase = (time - time.floor()) * SINC_PHASES as f64;
        let lower = phase.floor() as usize;
        let fraction = (phase - lower as f64) as f32;
        let desired_start = first.max(0) as u64;
        let desired_end = (first + sinc_len as i64 + 1).max(0) as u64;
        while self.frames_read <= desired_end && self.frames_read < segment.frames {
            if self
                .packet
                .as_ref()
                .is_none_or(|p| self.packet_frame >= p.frames as usize)
            {
                if stream.ordinal >= segment.end_packet {
                    return Err("meeting_alignment_truncated".into());
                }
                self.packet = Some(stream.next()?.ok_or("meeting_alignment_truncated")?);
                self.packet_frame = 0;
            }
            // Centered FIR support must not use startup/drain audio outside the
            // user-visible common interval, even when the original raw packet
            // contains those samples. Zero before convolution, not merely after.
            let value = if (self.frames_read as f64) < self.input_start
                || (self.frames_read as f64) >= self.input_end
            {
                0.0
            } else {
                decode_mono(
                    self.packet.as_ref().ok_or("meeting_alignment_state")?,
                    self.packet_frame,
                )?
            };
            self.packet_frame += 1;
            if self.frames_read >= desired_start {
                self.cache.push_back(value)
            } else {
                self.cache_start = self.frames_read + 1;
            }
            self.frames_read += 1;
            while self.cache_start < desired_start && !self.cache.is_empty() {
                self.cache.pop_front();
                self.cache_start += 1;
            }
        }
        while self.cache_start < desired_start && !self.cache.is_empty() {
            self.cache.pop_front();
            self.cache_start += 1;
        }
        for (offset, value) in self.support.iter_mut().enumerate() {
            *value = 0.0;
            let position = first + offset as i64;
            if position >= 0 && (position as u64) < segment.frames {
                let index = (position as u64)
                    .checked_sub(self.cache_start)
                    .ok_or("meeting_alignment_window")?;
                *value = *self
                    .cache
                    .get(index as usize)
                    .ok_or("meeting_alignment_window")?;
            }
        }
        let left = self.filter.get_sinc_interpolated(&self.support, 0, lower);
        let right = if lower + 1 == SINC_PHASES {
            self.filter.get_sinc_interpolated(&self.support, 1, 0)
        } else {
            self.filter
                .get_sinc_interpolated(&self.support, 0, lower + 1)
        };
        let output = left + (right - left) * fraction;
        if !output.is_finite() {
            return Err("meeting_alignment_sample_range".into());
        }
        Ok(output)
    }
}

#[cfg(test)]
#[path = "alignment_tests.rs"]
mod tests;
