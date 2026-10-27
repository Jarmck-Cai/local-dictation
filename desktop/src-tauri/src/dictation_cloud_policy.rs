//! Clock-driven cloud circuit policy without platform, network or credentials.

pub(crate) const COOLDOWN_MS: u64 = 10 * 60 * 1_000;

/// Give short mixed-language utterances 3 s; add 10 ms for each character
/// beyond 600, capped at 8 s. UTF-8 byte length would penalize Chinese.
pub(crate) fn cleanup_budget_ms(raw: &str) -> u64 {
    let length = raw.chars().filter(|c| !c.is_whitespace()).count() as u64;
    3_000 + length.saturating_sub(600).saturating_mul(10).min(5_000)
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct CloudTicket {
    generation: u64,
    blocked: Option<&'static str>,
    allow_recovery: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct CloudPermit {
    generation: u64,
    health_epoch: u64,
    half_open: bool,
}

#[derive(Clone, Copy)]
pub(crate) enum Completion {
    Success,
    TransportFailure,
    /// A response arrived, but its HTTP status or text was unacceptable.
    ResponseRejected,
}

#[derive(Default)]
pub(crate) struct CloudGate {
    pub(crate) manual_paused: bool,
    pub(crate) consecutive_failures: u32,
    cooldown_until_ms: Option<u64>,
    half_open: bool,
    generation: u64,
    health_epoch: u64,
}

impl CloudGate {
    pub(crate) fn remaining_ms(&self, now_ms: u64) -> u64 {
        self.cooldown_until_ms.unwrap_or(0).saturating_sub(now_ms)
    }

    pub(crate) fn in_cooldown(&self) -> bool {
        self.cooldown_until_ms.is_some()
    }

    /// Explicit resume clears automatic cooldown for future recordings, while
    /// invalidating tickets already captured before that user action.
    pub(crate) fn set_manual_paused(&mut self, paused: bool) -> bool {
        let invalidate = (paused && !self.manual_paused)
            || (!paused && (self.manual_paused || self.in_cooldown()));
        if invalidate {
            self.generation = self.generation.wrapping_add(1);
            self.health_epoch = self.health_epoch.wrapping_add(1);
            self.half_open = false;
        }
        self.manual_paused = paused;
        if !paused {
            self.cooldown_until_ms = None;
            self.consecutive_failures = 0;
        }
        invalidate
    }

    /// Capture at recording start, not when ASR completes. A blocked recording
    /// can never turn into a cloud request just because the user resumes later.
    pub(crate) fn capture(&self, now_ms: u64, normal_recording: bool) -> CloudTicket {
        let blocked = if self.manual_paused {
            Some("cloud_paused")
        } else if self.cooldown_until_ms.is_some()
            && (self.remaining_ms(now_ms) > 0 || !normal_recording || self.half_open)
        {
            Some("cloud_cooldown")
        } else {
            None
        };
        CloudTicket {
            generation: self.generation,
            blocked,
            allow_recovery: normal_recording,
        }
    }

    pub(crate) fn begin(
        &mut self,
        ticket: CloudTicket,
        now_ms: u64,
    ) -> Result<CloudPermit, &'static str> {
        if let Some(reason) = ticket.blocked {
            return Err(reason);
        }
        if self.manual_paused || ticket.generation != self.generation {
            return Err("cloud_paused_during_cleanup");
        }
        let half_open = self.cooldown_until_ms.is_some();
        if half_open {
            if self.remaining_ms(now_ms) > 0 || self.half_open || !ticket.allow_recovery {
                return Err("cloud_cooldown");
            }
            // Acquire only when a request starts: canceling a recording before
            // ASR must not reserve the half-open probe forever.
            self.half_open = true;
        }
        Ok(CloudPermit {
            generation: self.generation,
            health_epoch: self.health_epoch,
            half_open,
        })
    }

    pub(crate) fn finish(&mut self, permit: CloudPermit, result: Completion, now_ms: u64) {
        if !self.current(permit) {
            return;
        }
        match result {
            Completion::TransportFailure => {
                self.consecutive_failures = self.consecutive_failures.saturating_add(1);
                if permit.half_open || self.consecutive_failures >= 3 {
                    self.cooldown_until_ms = Some(now_ms.saturating_add(COOLDOWN_MS));
                    self.half_open = false;
                    // Ignore other requests which started before this opening.
                    self.health_epoch = self.health_epoch.wrapping_add(1);
                }
            }
            Completion::Success | Completion::ResponseRejected => {
                self.consecutive_failures = 0;
                self.cooldown_until_ms = None;
                self.half_open = false;
                if permit.half_open {
                    self.health_epoch = self.health_epoch.wrapping_add(1);
                }
            }
        }
    }

    pub(crate) fn cancel(&mut self, permit: CloudPermit) {
        if self.current(permit) && permit.half_open {
            self.half_open = false;
        }
    }

    pub(crate) fn is_current(&self, permit: CloudPermit) -> bool {
        !self.manual_paused && self.current(permit)
    }

    fn current(&self, permit: CloudPermit) -> bool {
        permit.generation == self.generation && permit.health_epoch == self.health_epoch
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_transcript_waits_at_most_three_seconds() {
        assert_eq!(cleanup_budget_ms("请打开 Claude Code"), 3_000);
    }

    #[test]
    fn long_transcript_budget_is_bounded_and_uses_characters_not_utf8_bytes() {
        assert_eq!(cleanup_budget_ms(&"中".repeat(300)), 3_000);
        assert_eq!(cleanup_budget_ms(&"中".repeat(700)), 4_000);
        assert_eq!(cleanup_budget_ms(&"a".repeat(700)), 4_000);
        assert_eq!(cleanup_budget_ms(&"中".repeat(20_000)), 8_000);
    }

    fn fail(gate: &mut CloudGate, now: u64) {
        let ticket = gate.capture(now, true);
        let permit = gate.begin(ticket, now).unwrap();
        gate.finish(permit, Completion::TransportFailure, now);
    }

    fn open(gate: &mut CloudGate) {
        for now in 1..=3 {
            fail(gate, now);
        }
    }

    #[test]
    fn three_consecutive_transport_failures_open_for_ten_minutes() {
        let mut gate = CloudGate::default();
        fail(&mut gate, 1);
        fail(&mut gate, 2);
        assert_eq!(gate.remaining_ms(2), 0);
        fail(&mut gate, 3);
        assert_eq!(gate.consecutive_failures, 3);
        assert_eq!(gate.remaining_ms(3), COOLDOWN_MS);
        assert_eq!(
            gate.begin(gate.capture(4, true), 4).unwrap_err(),
            "cloud_cooldown"
        );
    }

    #[test]
    fn successful_or_rejected_responses_break_transport_failure_streak() {
        for result in [Completion::Success, Completion::ResponseRejected] {
            let mut gate = CloudGate::default();
            fail(&mut gate, 1);
            fail(&mut gate, 2);
            let permit = gate.begin(gate.capture(3, true), 3).unwrap();
            gate.finish(permit, result, 3);
            fail(&mut gate, 4);
            assert_eq!(gate.consecutive_failures, 1);
            assert_eq!(gate.remaining_ms(4), 0);
        }
    }

    #[test]
    fn expired_cooldown_allows_only_one_normal_recording_probe() {
        let mut gate = CloudGate::default();
        open(&mut gate);
        let now = 3 + COOLDOWN_MS;
        assert_eq!(
            gate.begin(gate.capture(now, false), now).unwrap_err(),
            "cloud_cooldown"
        );
        let first = gate.capture(now, true);
        let concurrent = gate.capture(now, true);
        let permit = gate.begin(first, now).unwrap();
        assert_eq!(gate.begin(concurrent, now).unwrap_err(), "cloud_cooldown");
        gate.finish(permit, Completion::Success, now + 1);
        assert_eq!(gate.consecutive_failures, 0);
        assert!(gate.begin(gate.capture(now + 1, false), now + 1).is_ok());
    }

    #[test]
    fn failed_probe_reopens_and_dropped_probe_does_not_wedge_recovery() {
        let mut gate = CloudGate::default();
        open(&mut gate);
        let now = 3 + COOLDOWN_MS;
        let canceled = gate.begin(gate.capture(now, true), now).unwrap();
        gate.cancel(canceled);
        let probe = gate.begin(gate.capture(now, true), now).unwrap();
        gate.finish(probe, Completion::TransportFailure, now + 1);
        assert_eq!(gate.remaining_ms(now + 1), COOLDOWN_MS);
    }

    #[test]
    fn resume_does_not_enable_recordings_started_while_paused_or_before_pause() {
        let mut gate = CloudGate::default();
        let before = gate.capture(0, true);
        gate.set_manual_paused(true);
        let during = gate.capture(0, true);
        gate.set_manual_paused(false);
        assert_eq!(
            gate.begin(before, 0).unwrap_err(),
            "cloud_paused_during_cleanup"
        );
        assert_eq!(gate.begin(during, 0).unwrap_err(), "cloud_paused");
        assert!(gate.begin(gate.capture(0, true), 0).is_ok());
    }

    #[test]
    fn recording_started_during_cooldown_stays_local_after_expiry() {
        let mut gate = CloudGate::default();
        open(&mut gate);
        let ticket = gate.capture(4, true);
        assert_eq!(
            gate.begin(ticket, COOLDOWN_MS + 5).unwrap_err(),
            "cloud_cooldown"
        );
    }

    #[test]
    fn late_success_cannot_clear_a_newly_opened_circuit_or_pause() {
        let mut gate = CloudGate::default();
        let late = gate.begin(gate.capture(0, true), 0).unwrap();
        open(&mut gate);
        gate.finish(late, Completion::Success, 4);
        assert_eq!(gate.remaining_ms(4), COOLDOWN_MS - 1);
        let now = COOLDOWN_MS + 3;
        let late_probe = gate.begin(gate.capture(now, true), now).unwrap();
        gate.set_manual_paused(true);
        gate.finish(late_probe, Completion::Success, now + 1);
        assert!(gate.manual_paused);
        assert_eq!(gate.consecutive_failures, 3);
    }

    #[test]
    fn explicit_resume_overrides_cooldown_only_for_future_recordings() {
        let mut gate = CloudGate::default();
        open(&mut gate);
        let during_cooldown = gate.capture(4, true);
        gate.set_manual_paused(false);
        assert_eq!(gate.remaining_ms(4), 0);
        assert_eq!(gate.consecutive_failures, 0);
        assert_eq!(
            gate.begin(during_cooldown, 5).unwrap_err(),
            "cloud_cooldown"
        );
        assert!(gate.begin(gate.capture(5, true), 5).is_ok());
    }

    #[test]
    fn explicit_resume_invalidates_a_pending_half_open_probe() {
        let mut gate = CloudGate::default();
        open(&mut gate);
        let now = 3 + COOLDOWN_MS;
        let probe = gate.begin(gate.capture(now, true), now).unwrap();
        assert!(gate.set_manual_paused(false));
        assert!(!gate.is_current(probe));
        gate.finish(probe, Completion::TransportFailure, now + 1);
        assert_eq!(gate.consecutive_failures, 0);
        assert_eq!(gate.remaining_ms(now + 1), 0);
    }
}
