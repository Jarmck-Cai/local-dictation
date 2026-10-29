//! A shared recording slot prevents dictation from muting a running meeting.
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[derive(Default)]
pub struct RecordingGate {
    held: AtomicBool,
}

pub struct RecordingPermit {
    gate: Arc<RecordingGate>,
}

impl RecordingGate {
    /// Return a unique slot, released automatically on completion or failure.
    pub fn try_claim(self: &Arc<Self>) -> Option<RecordingPermit> {
        self.held
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| RecordingPermit {
                gate: Arc::clone(self),
            })
    }
}

impl Drop for RecordingPermit {
    fn drop(&mut self) {
        self.gate.held.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::Barrier, thread};
    #[test]
    fn either_recording_excludes_the_other_until_its_guard_ends() {
        let gate = Arc::new(RecordingGate::default());
        let dictation = gate.try_claim().expect("ordinary dictation can start");
        assert!(
            gate.try_claim().is_none(),
            "meeting cannot share the microphone"
        );
        drop(dictation);
        let meeting = gate.try_claim().expect("meeting can start after dictation");
        assert!(
            gate.try_claim().is_none(),
            "dictation cannot mute a meeting"
        );
        drop(meeting);
        assert!(gate.try_claim().is_some());
    }
    #[test]
    fn simultaneous_starts_grant_exactly_one_recording() {
        let gate = Arc::new(RecordingGate::default());
        let barrier = Arc::new(Barrier::new(3));
        let threads: Vec<_> = (0..2)
            .map(|_| {
                let gate = gate.clone();
                let barrier = barrier.clone();
                thread::spawn(move || {
                    let permit = gate.try_claim();
                    barrier.wait();
                    barrier.wait();
                    permit.is_some()
                })
            })
            .collect();
        barrier.wait();
        barrier.wait();
        assert_eq!(
            threads
                .into_iter()
                .filter_map(|t| t.join().ok())
                .filter(|won| *won)
                .count(),
            1
        );
        assert!(gate.try_claim().is_some());
    }
    #[test]
    fn unwind_releases_the_slot() {
        let gate = Arc::new(RecordingGate::default());
        let owned = gate.clone();
        let _ = std::panic::catch_unwind(move || {
            let _permit = owned.try_claim().unwrap();
            panic!("synthetic failure");
        });
        assert!(gate.try_claim().is_some());
    }
}
