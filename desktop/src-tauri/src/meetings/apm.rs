//! Single-owner adapter for the complete WebRTC AEC3 native output.
use std::{
    ffi::{c_int, c_void},
    marker::PhantomData,
    ptr::NonNull,
    rc::Rc,
};

pub const FRAME_SAMPLES: usize = 160;
pub const LATENCY_SAMPLES: usize = 128;
#[link(name = "webrtc_apm", kind = "static")]
#[link(name = "winmm")]
extern "C" {
    fn meeting_apm_create(reserved: c_int) -> *mut c_void;
    fn meeting_apm_destroy(handle: *mut c_void);
    fn meeting_apm_reset(handle: *mut c_void) -> c_int;
    fn meeting_apm_process(
        handle: *mut c_void,
        capture: *const f32,
        capture_n: usize,
        render: *const f32,
        render_n: usize,
        output: *mut f32,
        output_n: usize,
    ) -> c_int;
}
pub struct MeetingApm {
    state: NonNull<c_void>,
    failed: bool,
    _thread: PhantomData<Rc<()>>,
}
impl MeetingApm {
    pub fn new() -> Result<Self, &'static str> {
        // SAFETY: reserved=0 selects the sole supported complete AEC3 output.
        let state = NonNull::new(unsafe { meeting_apm_create(0) }).ok_or("meeting_apm_init")?;
        Ok(Self {
            state,
            failed: false,
            _thread: PhantomData,
        })
    }
    /// Process one normalized 16 kHz/10 ms pair. Validation does not alter state
    /// or output. Any native processing failure poisons this epoch until reset.
    pub fn process(
        &mut self,
        capture: &[f32],
        render: &[f32],
        output: &mut [f32],
    ) -> Result<(), &'static str> {
        if self.failed {
            return Err("meeting_apm_failed");
        }
        if capture.len() != FRAME_SAMPLES
            || render.len() != FRAME_SAMPLES
            || output.len() != FRAME_SAMPLES
        {
            return Err("meeting_apm_frame");
        }
        if capture
            .iter()
            .chain(render)
            .any(|s| !s.is_finite() || s.abs() > 1.0)
        {
            return Err("meeting_apm_sample");
        }
        // SAFETY: this !Send/!Sync instance uniquely owns a live native handle;
        // the checked slices satisfy its exact C ABI dimensions. Native writes
        // output only on success and retains no slice pointers after the call.
        let code = unsafe {
            meeting_apm_process(
                self.state.as_ptr(),
                capture.as_ptr(),
                capture.len(),
                render.as_ptr(),
                render.len(),
                output.as_mut_ptr(),
                output.len(),
            )
        };
        if code != 0 {
            self.failed = true;
            return Err("meeting_apm_process");
        }
        Ok(())
    }
    /// Start a new capture epoch with a fresh adaptive filter. A failed reset
    /// leaves the adapter poisoned; the caller must end the processing job.
    pub fn reset(&mut self) -> Result<(), &'static str> {
        // SAFETY: the uniquely borrowed native handle remains live until Drop.
        if unsafe { meeting_apm_reset(self.state.as_ptr()) } != 0 {
            self.failed = true;
            return Err("meeting_apm_reset");
        }
        self.failed = false;
        Ok(())
    }
}
impl Drop for MeetingApm {
    fn drop(&mut self) {
        // SAFETY: state is owned only here and destroyed exactly once.
        unsafe { meeting_apm_destroy(self.state.as_ptr()) }
    }
}
#[cfg(test)]
#[path = "apm_tests.rs"]
mod tests;
