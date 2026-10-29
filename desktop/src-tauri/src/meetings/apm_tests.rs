use super::*;

fn noise(n: usize) -> Vec<f32> {
    let mut state = 0x4832_9201u32;
    (0..n)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state as i32 as f32 / i32::MAX as f32) * 0.05
        })
        .collect()
}

#[test]
fn invalid_rust_inputs_leave_output_and_adaptive_state_unchanged() {
    let mut actual = MeetingApm::new().unwrap();
    let mut control = MeetingApm::new().unwrap();
    let good = noise(160);
    let silent = [0.0; 160];
    let mut output = [7.0; 160];
    assert_eq!(
        actual.process(&good[..159], &silent, &mut output),
        Err("meeting_apm_frame")
    );
    for invalid in [f32::NAN, f32::INFINITY, -1.001, 1.001] {
        let mut bad = good.clone();
        bad[159] = invalid;
        assert_eq!(
            actual.process(&bad, &silent, &mut output),
            Err("meeting_apm_sample")
        );
        assert_eq!(output, [7.0; 160]);
    }
    actual.process(&good, &silent, &mut output).unwrap();
    let mut expected = [0.0; 160];
    control.process(&good, &silent, &mut expected).unwrap();
    assert_eq!(output, expected);
}

#[test]
fn real_c_abi_validates_the_entire_call_before_writing_or_advancing() {
    // Deliberately bypass Rust validation to exercise the shipped native contract.
    let mut instance = MeetingApm::new().unwrap();
    let mut control = MeetingApm::new().unwrap();
    let good = noise(160);
    let mut bad = good.clone();
    bad[159] = f32::NAN;
    let mut output = [9.0; 160];
    unsafe {
        for (capture, capture_n, render, render_n, out_n) in [
            (good.as_ptr(), 159, good.as_ptr(), 160, 160),
            (good.as_ptr(), 160, good.as_ptr(), 160, 159),
            (bad.as_ptr(), 160, good.as_ptr(), 160, 160),
            (good.as_ptr(), 160, bad.as_ptr(), 160, 160),
            (std::ptr::null(), 160, good.as_ptr(), 160, 160),
        ] {
            assert_eq!(
                meeting_apm_process(
                    instance.state.as_ptr(),
                    capture,
                    capture_n,
                    render,
                    render_n,
                    output.as_mut_ptr(),
                    out_n
                ),
                -101
            );
            assert_eq!(output, [9.0; 160]);
        }
        assert!(meeting_apm_create(1).is_null());
    }
    instance.process(&good, &good, &mut output).unwrap();
    let mut expected = [0.0; 160];
    control.process(&good, &good, &mut expected).unwrap();
    assert_eq!(output, expected);
}

#[test]
fn independent_near_noise_has_the_frozen_delay_and_reset_discards_previous_epoch() {
    let input = noise(3200);
    let mut state = MeetingApm::new().unwrap();
    let mut output = Vec::new();
    for frame in input.chunks(160).chain(std::iter::once(&[0.0; 160][..])) {
        let mut y = [0.0; 160];
        state.process(frame, &[0.0; 160], &mut y).unwrap();
        output.extend(y);
    }
    assert!(output[..128].iter().all(|s| s.abs() < 1e-7));
    let residual: f32 = input
        .iter()
        .zip(&output[128..])
        .map(|(a, b)| (a - b).abs())
        .sum();
    assert!(
        residual / (input.len() as f32) < 1e-5,
        "near noise error {residual}"
    );
    state.reset().unwrap();
    let mut fresh = MeetingApm::new().unwrap();
    for frame in input.chunks(160).take(4) {
        let mut a = [0.0; 160];
        let mut b = [0.0; 160];
        state.process(frame, &[0.0; 160], &mut a).unwrap();
        fresh.process(frame, &[0.0; 160], &mut b).unwrap();
        assert_eq!(a, b);
    }
}

#[test]
fn independent_impulses_keep_first_and_last_samples_after_one_frame_flush() {
    let mut state = MeetingApm::new().unwrap();
    let mut input = [0.0; 320];
    input[0] = 0.25;
    input[319] = -0.5;
    let mut output = Vec::new();
    for x in input.chunks(160).chain(std::iter::once(&[0.0; 160][..])) {
        let mut y = [0.0; 160];
        state.process(x, &[0.0; 160], &mut y).unwrap();
        output.extend(y);
    }
    assert!((output[128] - 0.25).abs() < 1e-6);
    assert!((output[447] + 0.5).abs() < 1e-6);
    assert_eq!(output.iter().filter(|x| x.abs() > 1e-6).count(), 2);
}
