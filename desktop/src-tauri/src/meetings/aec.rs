//! Stateful, bounded 16 kHz meeting echo cancellation using SpeexDSP 1.2.1.
//!
//! Inputs must already share a timestamp-aligned clock. This is not a drift
//! corrector or a voice gate. Keep one instance for an entire recording epoch;
//! changing storage/ASR chunks must not discard the adaptive filter. Raw source
//! audio is always stored separately.

use std::ffi::{c_int, c_void};
use std::ptr::NonNull;

pub const SAMPLE_RATE: u32 = 16_000;
pub const FRAME_SAMPLES: usize = 160;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AecError {
    InitializationFailed,
    InvalidFrameLength,
    InvalidSample,
}

// 320 ms covers the prescribed 120 ms delay plus the synthetic room tail.
const FILTER_SAMPLES: c_int = 5_120;
const SPEEX_ECHO_SET_SAMPLING_RATE: c_int = 24;

#[link(name = "meeting_speexdsp", kind = "static")]
extern "C" {
    fn speex_echo_state_init(frame_size: c_int, filter_length: c_int) -> *mut c_void;
    fn speex_echo_state_destroy(state: *mut c_void);
    fn speex_echo_state_reset(state: *mut c_void);
    fn speex_echo_ctl(state: *mut c_void, request: c_int, value: *mut c_void) -> c_int;
    fn speex_echo_cancellation(
        state: *mut c_void,
        near: *const i16,
        reference: *const i16,
        output: *mut i16,
    );
}

/// Owns one native adaptive filter. `&mut self` serializes all calls; no global
/// state, callbacks, audio devices, file access or network are used.
pub struct MeetingAec {
    state: NonNull<c_void>,
}

impl MeetingAec {
    pub fn new() -> Result<Self, AecError> {
        // SAFETY: fixed positive dimensions satisfy the upstream C API.
        let state =
            NonNull::new(unsafe { speex_echo_state_init(FRAME_SAMPLES as c_int, FILTER_SAMPLES) })
                .ok_or(AecError::InitializationFailed)?;
        let instance = Self { state };
        let mut rate = SAMPLE_RATE as c_int;
        // SAFETY: instance owns a live state; ctl takes an int pointer for 24.
        let result = unsafe {
            speex_echo_ctl(
                state.as_ptr(),
                SPEEX_ECHO_SET_SAMPLING_RATE,
                (&mut rate as *mut c_int).cast(),
            )
        };
        if result != 0 {
            return Err(AecError::InitializationFailed);
        }
        Ok(instance)
    }

    /// Process exactly one normalized mono 10 ms pair. Invalid input leaves
    /// native state and caller output untouched. Quantization is explicit; the
    /// internal upstream algorithm uses floating point and has no AGC/preprocessor.
    pub fn process(
        &mut self,
        near: &[f32],
        reference: &[f32],
        output: &mut [f32],
    ) -> Result<(), AecError> {
        if near.len() != FRAME_SAMPLES
            || reference.len() != FRAME_SAMPLES
            || output.len() != FRAME_SAMPLES
        {
            return Err(AecError::InvalidFrameLength);
        }
        if near
            .iter()
            .chain(reference)
            .any(|v| !v.is_finite() || !(-1.0..=1.0).contains(v))
        {
            return Err(AecError::InvalidSample);
        }
        let mut n = [0i16; FRAME_SAMPLES];
        let mut r = [0i16; FRAME_SAMPLES];
        let mut y = [0i16; FRAME_SAMPLES];
        for i in 0..FRAME_SAMPLES {
            n[i] = (near[i] * 32768.0).round().clamp(-32768.0, 32767.0) as i16;
            r[i] = (reference[i] * 32768.0).round().clamp(-32768.0, 32767.0) as i16;
        }
        // SAFETY: live exclusively borrowed state, and three non-overlapping
        // arrays with the native state's exact frame length.
        unsafe {
            speex_echo_cancellation(self.state.as_ptr(), n.as_ptr(), r.as_ptr(), y.as_mut_ptr());
        }
        for (out, value) in output.iter_mut().zip(y) {
            *out = value as f32 / 32768.0;
        }
        Ok(())
    }

    /// Begin a new epoch after a capture interruption or explicit restart.
    pub fn reset(&mut self) {
        // SAFETY: exclusively borrowed, live state.
        unsafe {
            speex_echo_state_reset(self.state.as_ptr());
        }
    }
}

impl Drop for MeetingAec {
    fn drop(&mut self) {
        // SAFETY: uniquely owned state is destroyed exactly once.
        unsafe {
            speex_echo_state_destroy(self.state.as_ptr());
        }
    }
}

#[cfg(test)]
#[path = "aec_tests.rs"]
mod tests;
