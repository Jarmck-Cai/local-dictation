//! Offline policies for a one-shot manual paste gesture.
use std::time::{Duration, Instant};

#[derive(Debug, PartialEq, Eq)]
pub enum ReleaseDecision {
    Pending,
    Ready,
    Cancelled,
    TimedOut,
}

pub struct ReleaseWait {
    deadline: Instant,
}
impl ReleaseWait {
    pub fn new(start: Instant, timeout: Duration) -> Self {
        Self {
            deadline: start + timeout,
        }
    }
    pub fn poll(&self, now: Instant, any_key_down: bool, cancelled: bool) -> ReleaseDecision {
        if cancelled {
            ReleaseDecision::Cancelled
        } else if now >= self.deadline {
            ReleaseDecision::TimedOut
        } else if any_key_down {
            ReleaseDecision::Pending
        } else {
            ReleaseDecision::Ready
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ManualOutcome {
    ReadObserved,
    InputSent,
    NoResult,
    Cancelled,
    KeysHeld,
    Unsupported,
    KeyboardUnavailable,
    HistoryFailed,
    Failed(String),
}

pub fn execute(
    is_cancelled: impl Fn() -> bool,
    wait_release: impl FnOnce() -> Result<(), ManualOutcome>,
    read_latest: impl FnOnce() -> Result<Option<String>, ()>,
    paste: impl FnOnce(String) -> Result<bool, String>,
) -> ManualOutcome {
    if is_cancelled() {
        return ManualOutcome::Cancelled;
    }
    if let Err(outcome) = wait_release() {
        return outcome;
    }
    if is_cancelled() {
        return ManualOutcome::Cancelled;
    }
    let body = match read_latest() {
        Ok(Some(body)) if !body.trim().is_empty() => body,
        Ok(_) => return ManualOutcome::NoResult,
        Err(()) => return ManualOutcome::HistoryFailed,
    };
    if is_cancelled() {
        return ManualOutcome::Cancelled;
    }
    match paste(body) {
        Ok(true) => ManualOutcome::ReadObserved,
        Ok(false) => ManualOutcome::InputSent,
        Err(code) if code == "cancelled" || code == "manual_paste_cancelled" => {
            ManualOutcome::Cancelled
        }
        Err(code) if code == "keys_held" => ManualOutcome::KeysHeld,
        Err(code) if code == "keyboard_unavailable" => ManualOutcome::KeyboardUnavailable,
        Err(code) if code == "unsupported" => ManualOutcome::Unsupported,
        Err(code) => ManualOutcome::Failed(code),
    }
}

/// One physical gesture, kept claimed across auto-repeat and partial releases.
pub struct GestureLatch(std::sync::atomic::AtomicU64, std::sync::atomic::AtomicU64);
impl GestureLatch {
    pub const fn new() -> Self {
        Self(
            std::sync::atomic::AtomicU64::new(0),
            std::sync::atomic::AtomicU64::new(0),
        )
    }
    pub fn begin_release_watch(&self, ticket: u64) -> bool {
        use std::sync::atomic::Ordering;
        if self.current() != Some(ticket) {
            return false;
        }
        let observed = self.1.load(Ordering::Acquire);
        observed != ticket
            && self
                .1
                .compare_exchange(observed, ticket, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
    }
    pub fn finish_release_watch(&self, ticket: u64) {
        use std::sync::atomic::Ordering;
        let _ = self
            .1
            .compare_exchange(ticket, 0, Ordering::AcqRel, Ordering::Acquire);
    }
    pub fn claim(&self) -> Option<u64> {
        use std::sync::atomic::Ordering;
        let idle = self.0.load(Ordering::Acquire);
        if idle & 1 != 0 {
            return None;
        }
        let ticket = idle.wrapping_add(1);
        self.0
            .compare_exchange(idle, ticket, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| ticket)
    }
    pub fn current(&self) -> Option<u64> {
        let ticket = self.0.load(std::sync::atomic::Ordering::Acquire);
        (ticket & 1 != 0).then_some(ticket)
    }
    pub fn release(&self, ticket: u64) {
        use std::sync::atomic::Ordering;
        let _ = self.0.compare_exchange(
            ticket,
            ticket.wrapping_add(1),
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn held_modifier_prevents_paste_until_all_keys_are_up() {
        let start = Instant::now();
        let wait = ReleaseWait::new(start, Duration::from_secs(2));
        assert_eq!(wait.poll(start, true, false), ReleaseDecision::Pending);
        assert_eq!(
            wait.poll(start + Duration::from_millis(120), true, false),
            ReleaseDecision::Pending
        );
        assert_eq!(
            wait.poll(start + Duration::from_millis(130), false, false),
            ReleaseDecision::Ready
        );
    }
    #[test]
    fn release_at_or_after_deadline_never_pastes_late() {
        let start = Instant::now();
        let wait = ReleaseWait::new(start, Duration::from_secs(2));
        assert_eq!(
            wait.poll(start + Duration::from_secs(2), false, false),
            ReleaseDecision::TimedOut
        );
        assert_eq!(
            wait.poll(start + Duration::from_secs(3), false, false),
            ReleaseDecision::TimedOut
        );
    }
    #[test]
    fn cancel_wins_over_release() {
        let start = Instant::now();
        let wait = ReleaseWait::new(start, Duration::from_secs(2));
        assert_eq!(wait.poll(start, false, true), ReleaseDecision::Cancelled);
        assert_eq!(wait.poll(start, true, true), ReleaseDecision::Cancelled);
    }
    #[test]
    fn cancellation_before_start_never_reads_or_pastes() {
        assert_eq!(
            execute(
                || true,
                || panic!("must not wait"),
                || panic!("must not read"),
                |_| panic!("must not paste")
            ),
            ManualOutcome::Cancelled
        );
    }
    #[test]
    fn held_key_timeout_never_reads_or_pastes() {
        assert_eq!(
            execute(
                || false,
                || Err(ManualOutcome::KeysHeld),
                || panic!("must not read"),
                |_| panic!("must not paste")
            ),
            ManualOutcome::KeysHeld
        );
    }
    #[test]
    fn cancellation_during_history_read_never_pastes() {
        use std::cell::Cell;
        let cancelled = Cell::new(false);
        let result = execute(
            || cancelled.get(),
            || Ok(()),
            || {
                cancelled.set(true);
                Ok(Some("synthetic final".into()))
            },
            |_| panic!("must not paste"),
        );
        assert_eq!(result, ManualOutcome::Cancelled);
    }
    #[test]
    fn empty_or_failed_history_never_pastes() {
        assert_eq!(
            execute(
                || false,
                || Ok(()),
                || Ok(None),
                |_| panic!("must not paste")
            ),
            ManualOutcome::NoResult
        );
        assert_eq!(
            execute(
                || false,
                || Ok(()),
                || Ok(Some("  ".into())),
                |_| panic!("must not paste")
            ),
            ManualOutcome::NoResult
        );
        assert_eq!(
            execute(
                || false,
                || Ok(()),
                || Err(()),
                |_| panic!("must not paste")
            ),
            ManualOutcome::HistoryFailed
        );
    }
    #[test]
    fn one_paste_preserves_final_body_and_reports_observed_read_only() {
        let outcome = execute(
            || false,
            || Ok(()),
            || Ok(Some("synthetic FINAL".into())),
            |body| {
                assert_eq!(body, "synthetic FINAL");
                Ok(true)
            },
        );
        assert_eq!(outcome, ManualOutcome::ReadObserved);
        assert_eq!(
            execute(
                || false,
                || Ok(()),
                || Ok(Some("final".into())),
                |_| Ok(false)
            ),
            ManualOutcome::InputSent
        );
    }
    #[test]
    fn failed_paste_is_attempted_once_without_retry() {
        use std::cell::Cell;
        let attempts = Cell::new(0);
        let result = execute(
            || false,
            || Ok(()),
            || Ok(Some("final".into())),
            |_| {
                attempts.set(attempts.get() + 1);
                Err("backup_failed".into())
            },
        );
        assert_eq!(attempts.get(), 1);
        assert_eq!(result, ManualOutcome::Failed("backup_failed".into()));
    }
    #[test]
    fn busy_gesture_stays_consumed_until_physical_release() {
        let latch = GestureLatch::new();
        let ticket = latch.claim().expect("first press must claim");
        assert!(latch.claim().is_none());
        // Busy denial and a partial combo release do not release the ticket.
        assert_eq!(latch.current(), Some(ticket));
        assert!(latch.claim().is_none());
        latch.release(ticket);
        assert!(latch.claim().is_some());
    }
    #[test]
    fn late_release_worker_cannot_rearm_a_new_gesture() {
        let latch = GestureLatch::new();
        let first = latch.claim().unwrap();
        latch.release(first);
        let second = latch.claim().unwrap();
        latch.release(first);
        assert_eq!(latch.current(), Some(second));
        assert!(latch.claim().is_none());
        latch.release(second);
        assert!(latch.claim().is_some());
    }
    #[test]
    fn timed_out_gesture_recovers_with_one_release_watch_and_no_new_claim() {
        let latch = GestureLatch::new();
        let ticket = latch.claim().unwrap();
        assert!(latch.begin_release_watch(ticket));
        assert!(
            !latch.begin_release_watch(ticket),
            "repeat must not create another waiter"
        );
        latch.finish_release_watch(ticket); // first bounded waiter timed out
        assert!(
            latch.claim().is_none(),
            "held gesture never becomes a paste request"
        );
        assert!(
            latch.begin_release_watch(ticket),
            "next edge can watch release only"
        );
        assert!(!latch.begin_release_watch(ticket));
        latch.release(ticket); // F9 up followed by both modifiers up
        latch.finish_release_watch(ticket);
        assert!(
            latch.claim().is_some(),
            "all-up restores the next complete gesture"
        );
    }
    #[test]
    fn old_watch_finish_does_not_release_the_new_gesture_watcher() {
        let latch = GestureLatch::new();
        let first = latch.claim().unwrap();
        assert!(latch.begin_release_watch(first));
        latch.release(first);
        let second = latch.claim().unwrap();
        assert!(latch.begin_release_watch(second));
        latch.finish_release_watch(first);
        assert!(!latch.begin_release_watch(second));
    }
    #[test]
    fn protected_cancel_is_cancelled_but_restoration_failure_is_reported() {
        assert_eq!(
            execute(
                || false,
                || Ok(()),
                || Ok(Some("final".into())),
                |_| Err("manual_paste_cancelled".into())
            ),
            ManualOutcome::Cancelled
        );
        assert_eq!(
            execute(
                || false,
                || Ok(()),
                || Ok(Some("final".into())),
                |_| Err("manual_paste_restore_failed".into())
            ),
            ManualOutcome::Failed("manual_paste_restore_failed".into())
        );
    }
    #[test]
    fn final_keyboard_gate_keeps_existing_ui_categories() {
        for (code, want) in [
            ("keys_held", ManualOutcome::KeysHeld),
            ("keyboard_unavailable", ManualOutcome::KeyboardUnavailable),
            ("unsupported", ManualOutcome::Unsupported),
        ] {
            assert_eq!(
                execute(
                    || false,
                    || Ok(()),
                    || Ok(Some("final".into())),
                    |_| Err(code.into())
                ),
                want
            );
        }
    }
}
