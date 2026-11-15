use super::*;

// These exercise the production wrapper. Passthrough breaks echo reduction;
// zero output breaks near-end preservation, including silence transitions.
fn signal(samples: usize, seed: u32) -> Vec<f32> {
    let mut state = seed;
    let mut low = 0.0;
    (0..samples)
        .map(|_| {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            let white = ((state >> 8) as f32 / 16777216.0 - 0.5) * 0.6;
            low = 0.6 * low + 0.4 * white;
            low
        })
        .collect()
}

fn run(near: &[f32], reference: &[f32]) -> Vec<f32> {
    let mut aec = MeetingAec::new().unwrap();
    let mut result = vec![0.0; near.len()];
    for ((n, r), y) in near
        .chunks_exact(160)
        .zip(reference.chunks_exact(160))
        .zip(result.chunks_exact_mut(160))
    {
        aec.process(n, r, y).unwrap();
    }
    result
}

fn power(x: &[f32]) -> f64 {
    x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len() as f64
}
fn correlation(a: &[f32], b: &[f32]) -> f64 {
    let dot: f64 = a.iter().zip(b).map(|(a, b)| *a as f64 * *b as f64).sum();
    dot / (power(a) * power(b)).sqrt() / a.len() as f64
}

// Independent direct-form transfer function from the published SpeexDSP 1.2.1
// filter_dc_notch16. This never calls the native AEC, estimates a filter, or
// searches a delay. The old dry-waveform gate remains recorded in aec/.
fn fixed_notch(input: &[f32], quantize: bool) -> Vec<f32> {
    let radius = 0.982_f64;
    let denominator = radius * radius + 0.7 * (1.0 - radius).powi(2);
    let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
    input
        .iter()
        .map(|sample| {
            let x = if quantize {
                (sample * 32768.0).round().clamp(-32768.0, 32767.0) as f64 / 32768.0
            } else {
                *sample as f64
            };
            let y = radius * (x - 2.0 * x1 + x2) + 2.0 * radius * y1 - denominator * y2;
            (x2, x1, y2, y1) = (x1, x, y1, y);
            if quantize {
                // arch.h WORD2INT uses floor(.5+x), including negative ties.
                ((y * 32768.0 + 0.5).floor().clamp(-32768.0, 32767.0) / 32768.0) as f32
            } else {
                y as f32
            }
        })
        .collect()
}

#[test]
fn independent_notch_matches_analytic_frequency_response() {
    let mut impulse = vec![0.0; 16_384];
    impulse[0] = 1.0;
    let response = fixed_notch(&impulse, false);
    for hz in [
        0.0_f64, 20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 4000.0, 7900.0,
    ] {
        let w = std::f64::consts::TAU * hz / 16_000.0;
        let (mut re, mut im) = (0.0, 0.0);
        for (i, sample) in response.iter().enumerate() {
            re += *sample as f64 * (w * i as f64).cos();
            im -= *sample as f64 * (w * i as f64).sin();
        }
        let r = 0.982_f64;
        let a2 = r * r + 0.7 * (1.0 - r).powi(2);
        let nr = r * (1.0 - 2.0 * w.cos() + (2.0 * w).cos());
        let ni = r * (2.0 * w.sin() - (2.0 * w).sin());
        let dr = 1.0 - 2.0 * r * w.cos() + a2 * (2.0 * w).cos();
        let di = 2.0 * r * w.sin() - a2 * (2.0 * w).sin();
        let expected_re = (nr * dr + ni * di) / (dr * dr + di * di);
        let expected_im = (ni * dr - nr * di) / (dr * dr + di * di);
        assert!((re - expected_re).abs() < 2e-7, "real response at {hz} Hz");
        assert!(
            (im - expected_im).abs() < 2e-7,
            "imaginary response at {hz} Hz"
        );
    }
}

#[test]
fn converged_echo_is_reduced_by_at_least_fifteen_db() {
    let reference = signal(16_000 * 12, 0x715ab);
    let mut echo = vec![0.0; reference.len()];
    for i in 691..echo.len() {
        echo[i] = 0.55 * reference[i - 640] + 0.2 * reference[i - 691];
    }
    let out = run(&echo, &reference);
    let tail = 16_000 * 9;
    let erle = 10.0 * (power(&echo[tail..]) / power(&out[tail..]).max(1e-16)).log10();
    assert!(erle >= 15.0, "tail ERLE {erle:.3} dB < 15 dB");
}

#[test]
fn near_only_preserves_fixed_notch_waveform_and_dry_energy() {
    let near = signal(16_000 * 3, 0x2a77);
    let out = run(&near, &vec![0.0; near.len()]);
    let expected = fixed_notch(&near, true);
    let corr = correlation(&expected[1600..], &out[1600..]);
    let gain = 10.0 * (power(&out[1600..]) / power(&near[1600..])).log10();
    assert!(
        corr >= 0.99,
        "near-only correlation against independent H {corr}"
    );
    assert!(gain.abs() <= 1.0, "near-only energy {gain} dB");
}

#[test]
fn invalid_frames_preserve_an_already_adapted_state() {
    let input = signal(160 * 51, 0xd0e41);
    let mut tested = MeetingAec::new().unwrap();
    let mut control = MeetingAec::new().unwrap();
    let mut a = [0.0; 160];
    let mut b = [0.0; 160];
    for frame in input[..160 * 50].chunks_exact(160) {
        tested.process(frame, frame, &mut a).unwrap();
        control.process(frame, frame, &mut b).unwrap();
    }
    let before = a;
    for bad_value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1.0001, -1.0001] {
        let mut bad = [0.0; 160];
        bad[90] = bad_value;
        assert_eq!(
            tested.process(&bad, &[0.0; 160], &mut a),
            Err(AecError::InvalidSample)
        );
        assert_eq!(
            tested.process(&[0.0; 160], &bad, &mut a),
            Err(AecError::InvalidSample)
        );
        assert_eq!(a, before);
    }
    assert_eq!(
        tested.process(&[0.0; 160], &[0.0; 159], &mut a),
        Err(AecError::InvalidFrameLength)
    );
    assert_eq!(a, before);
    let mut short_output = [0.123; 159];
    assert_eq!(
        tested.process(&[0.0; 160], &[0.0; 160], &mut short_output),
        Err(AecError::InvalidFrameLength)
    );
    assert!(short_output.iter().all(|v| *v == 0.123));
    tested
        .process(&input[160 * 50..], &[0.0; 160], &mut a)
        .unwrap();
    control
        .process(&input[160 * 50..], &[0.0; 160], &mut b)
        .unwrap();
    assert_eq!(a, b);
}

#[test]
fn invalid_frames_do_not_advance_or_write_state() {
    let mut aec = MeetingAec::new().unwrap();
    let mut output = [0.123; 160];
    assert_eq!(
        aec.process(&[0.0; 159], &[0.0; 160], &mut output),
        Err(AecError::InvalidFrameLength)
    );
    assert!(output.iter().all(|v| *v == 0.123));
    let mut bad = [0.0; 160];
    bad[10] = f32::NAN;
    assert_eq!(
        aec.process(&bad, &[0.0; 160], &mut output),
        Err(AecError::InvalidSample)
    );
    assert!(output.iter().all(|v| *v == 0.123));
    bad[10] = 1.01;
    assert_eq!(
        aec.process(&[0.0; 160], &bad, &mut output),
        Err(AecError::InvalidSample)
    );
    let valid = signal(160, 42);
    aec.process(&valid, &[0.0; 160], &mut output).unwrap();
    let mut fresh = MeetingAec::new().unwrap();
    let mut expected = [0.0; 160];
    fresh.process(&valid, &[0.0; 160], &mut expected).unwrap();
    assert_eq!(output, expected);
}

#[test]
fn reset_returns_to_fresh_epoch() {
    let mut aec = MeetingAec::new().unwrap();
    let mut fresh = MeetingAec::new().unwrap();
    let input = signal(160 * 80, 9981);
    let mut a = [0.0; 160];
    let mut b = [0.0; 160];
    for frame in input.chunks_exact(160) {
        aec.process(frame, frame, &mut a).unwrap();
    }
    aec.reset();
    aec.process(&input[..160], &[0.0; 160], &mut a).unwrap();
    fresh.process(&input[..160], &[0.0; 160], &mut b).unwrap();
    assert_eq!(a, b);
}

#[test]
fn digital_silence_remains_silent() {
    let out = run(&[0.0; 1600], &[0.0; 1600]);
    assert!(out.iter().all(|v| *v == 0.0));
}
