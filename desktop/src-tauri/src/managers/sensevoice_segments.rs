//! Lossless, bounded batch segmentation for the 16 kHz SenseVoice input.

use std::ops::Range;

pub(super) const SAMPLE_RATE: usize = 16_000;
pub(super) const WHOLE_RECORDING_SAMPLES: usize = SAMPLE_RATE * 20;
const MAX_SEGMENT_SAMPLES: usize = SAMPLE_RATE * 15;
const MIN_SEGMENT_SAMPLES: usize = SAMPLE_RATE * 5;
const MIN_PAUSE_SAMPLES: usize = SAMPLE_RATE * 350 / 1000;

pub(super) struct FrameActivity<'a> {
    pub frame_samples: usize,
    pub speech: &'a [bool],
}

pub(super) fn segment_ranges(
    sample_count: usize,
    activity: Option<FrameActivity<'_>>,
) -> Vec<Range<usize>> {
    if sample_count == 0 {
        return Vec::new();
    }
    if sample_count <= WHOLE_RECORDING_SAMPLES {
        return vec![0..sample_count];
    }

    // VAD only chooses boundaries. Even a misclassified silent frame remains
    // in exactly one segment, so no content can disappear during segmentation.
    let mut pauses = Vec::new();
    if let Some(activity) = activity.filter(|activity| {
        activity.frame_samples > 0
            && activity.speech.len() == sample_count.div_ceil(activity.frame_samples)
    }) {
        let mut pause_start = None;
        for (frame_index, speech) in activity
            .speech
            .iter()
            .copied()
            .chain(std::iter::once(true))
            .enumerate()
        {
            let position = frame_index
                .saturating_mul(activity.frame_samples)
                .min(sample_count);
            if speech {
                if let Some(start) = pause_start.take() {
                    if position - start >= MIN_PAUSE_SAMPLES {
                        pauses.push(start..position);
                    }
                }
            } else if pause_start.is_none() {
                pause_start = Some(position);
            }
        }
    }

    let mut ranges = Vec::new();
    let mut start = 0;
    while sample_count - start > MAX_SEGMENT_SAMPLES {
        let remaining = sample_count - start;
        // Avoid a very short last segment (for example, 15s + 7 samples).
        let hard_end = start
            + if remaining < MAX_SEGMENT_SAMPLES + MIN_SEGMENT_SAMPLES {
                remaining / 2
            } else {
                MAX_SEGMENT_SAMPLES
            };
        let earliest = start + MIN_SEGMENT_SAMPLES;
        let end = pauses
            .iter()
            .filter_map(|pause| {
                let quiet_start = pause.start.max(earliest);
                let quiet_end = pause.end.min(hard_end);
                (quiet_end.saturating_sub(quiet_start) >= MIN_PAUSE_SAMPLES)
                    .then(|| quiet_start + (quiet_end - quiet_start) / 2)
            })
            .next_back()
            .unwrap_or(hard_end);
        ranges.push(start..end);
        start = end;
    }
    ranges.push(start..sample_count);
    ranges
}

pub(super) fn collect_frame_activity<E>(
    samples: &[f32],
    frame_samples: usize,
    mut detect: impl FnMut(&[f32]) -> Result<bool, E>,
) -> Result<Vec<bool>, E> {
    let mut flags = Vec::with_capacity(samples.len().div_ceil(frame_samples));
    for frame in samples.chunks(frame_samples) {
        if frame.len() == frame_samples {
            flags.push(detect(frame)?);
        } else {
            let mut padded = vec![0.0; frame_samples];
            padded[..frame.len()].copy_from_slice(frame);
            flags.push(detect(&padded)?);
        }
    }
    Ok(flags)
}

pub(super) fn decode_segments<E>(
    samples: &[f32],
    ranges: &[Range<usize>],
    mut decode: impl FnMut(usize, &[f32]) -> Result<String, E>,
) -> Result<String, E> {
    let mut texts = Vec::new();
    for (index, range) in ranges.iter().enumerate() {
        let text = decode(index, &samples[range.clone()])?;
        let text = text.trim();
        if !text.is_empty() {
            texts.push(text.to_string());
        }
    }
    Ok(texts.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_recording_keeps_whole_path_at_twenty_seconds() {
        assert_eq!(segment_ranges(320_000, None), vec![0..320_000]);
        assert!(segment_ranges(0, None).is_empty());
    }

    #[test]
    fn long_recording_without_vad_keeps_every_sample_in_bounded_segments() {
        assert_eq!(
            segment_ranges(960_007, None),
            vec![
                0..240_000,
                240_000..480_000,
                480_000..720_000,
                720_000..840_003,
                840_003..960_007
            ]
        );
    }

    #[test]
    fn long_recording_prefers_a_real_pause_over_a_hard_speech_boundary() {
        // 30 seconds; a half-second pause at 12 seconds. Every sample of the
        // pause must remain in exactly one neighboring segment.
        let mut speech = vec![true; 1_000];
        speech[400..417].fill(false);
        assert_eq!(
            segment_ranges(
                480_000,
                Some(FrameActivity {
                    frame_samples: 480,
                    speech: &speech
                })
            ),
            vec![0..196_080, 196_080..338_040, 338_040..480_000]
        );
    }

    #[test]
    fn multiple_pauses_choose_the_latest_usable_boundary() {
        let mut speech = vec![true; 1_000];
        speech[300..320].fill(false);
        speech[450..470].fill(false);
        assert_eq!(
            segment_ranges(
                480_000,
                Some(FrameActivity {
                    frame_samples: 480,
                    speech: &speech
                })
            ),
            vec![0..220_800, 220_800..350_400, 350_400..480_000]
        );
    }

    #[test]
    fn final_vad_frame_is_zero_padded_without_changing_original_audio() {
        let samples = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let mut observed = Vec::new();
        let flags = collect_frame_activity(&samples, 4, |frame| {
            observed.push(frame.to_vec());
            Ok::<_, ()>(true)
        })
        .unwrap();
        assert_eq!(flags, vec![true, true]);
        assert_eq!(
            observed,
            vec![vec![1.0, 2.0, 3.0, 4.0], vec![5.0, 0.0, 0.0, 0.0]]
        );
        assert_eq!(samples, vec![1.0, 2.0, 3.0, 4.0, 5.0]);
    }

    #[test]
    fn decoding_visits_every_segment_in_order_including_tail() {
        let samples = vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let text = decode_segments(&samples, &[0..3, 3..6, 6..7], |_, part| {
            Ok::<_, ()>(format!("{}-{}", part[0], part[part.len() - 1]))
        })
        .unwrap();
        assert_eq!(text, "0-2 3-5 6-6");
    }

    #[test]
    fn a_failed_segment_returns_an_error_instead_of_inserting_partial_text() {
        let samples = vec![1.0, 2.0, 3.0];
        let mut visited = Vec::new();
        let text = decode_segments(&samples, &[0..1, 1..2, 2..3], |index, part| {
            visited.push(part[0]);
            if index == 1 {
                Err("segment failed")
            } else {
                Ok("first".to_string())
            }
        });
        assert_eq!(text, Err("segment failed"));
        assert_eq!(visited, vec![1.0, 2.0]);
    }

    #[test]
    fn continuous_speech_at_threshold_and_three_minutes_never_drops_the_tail() {
        for sample_count in [320_001_usize, 480_001, 960_007, 2_880_000] {
            let speech = vec![true; sample_count.div_ceil(480)];
            let ranges = segment_ranges(
                sample_count,
                Some(FrameActivity {
                    frame_samples: 480,
                    speech: &speech,
                }),
            );
            let original: Vec<f32> = (0..sample_count).map(|index| index as f32).collect();
            let decoded: Vec<f32> = ranges
                .iter()
                .flat_map(|range| original[range.clone()].iter().copied())
                .collect();
            assert_eq!(decoded, original);
            assert!(ranges.iter().all(|range| range.len() <= 240_000));
            assert!(ranges.iter().all(|range| range.len() >= 80_000));
        }
    }

    #[test]
    fn invalid_or_entirely_silent_vad_never_discards_audio() {
        let malformed = [false; 3];
        assert_eq!(
            segment_ranges(
                480_000,
                Some(FrameActivity {
                    frame_samples: 480,
                    speech: &malformed
                })
            ),
            vec![0..240_000, 240_000..480_000]
        );
        assert_eq!(
            segment_ranges(
                480_000,
                Some(FrameActivity {
                    frame_samples: 0,
                    speech: &[]
                })
            ),
            vec![0..240_000, 240_000..480_000]
        );
        let silent = vec![false; 1_000];
        let ranges = segment_ranges(
            480_000,
            Some(FrameActivity {
                frame_samples: 480,
                speech: &silent,
            }),
        );
        assert_eq!(ranges.first().map(|range| range.start), Some(0));
        assert_eq!(ranges.last().map(|range| range.end), Some(480_000));
        assert_eq!(
            ranges.iter().map(|range| range.len()).sum::<usize>(),
            480_000
        );
        assert!(ranges.iter().all(|range| range.len() <= 240_000));
    }

    #[test]
    fn brief_noise_frames_do_not_cut_continuous_speech() {
        let mut speech = vec![true; 1_000];
        speech[400..403].fill(false);
        assert_eq!(
            segment_ranges(
                480_000,
                Some(FrameActivity {
                    frame_samples: 480,
                    speech: &speech
                })
            ),
            vec![0..240_000, 240_000..480_000]
        );
    }

    #[test]
    fn vad_failure_is_visible_to_the_caller_for_bounded_fallback() {
        let mut frames = 0;
        let result = collect_frame_activity(&[1.0; 12], 4, |_| {
            frames += 1;
            if frames == 2 {
                Err("vad failed")
            } else {
                Ok(true)
            }
        });
        assert_eq!(result, Err("vad failed"));
        assert_eq!(frames, 2);
    }

    #[test]
    fn empty_segment_outputs_do_not_hide_later_results_or_add_whitespace() {
        let outputs = [" first ", " ", " tail "];
        let result = decode_segments(&[0.0; 3], &[0..1, 1..2, 2..3], |index, _| {
            Ok::<_, ()>(outputs[index].to_string())
        });
        assert_eq!(result, Ok("first tail".to_string()));
    }
}
