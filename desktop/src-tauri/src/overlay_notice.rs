//! Sticky failure visibility decisions, independent of any windowing runtime.
#[derive(Default)]
pub struct OverlayNotice {
    failures: u8,
    generation: u64,
    progress: Option<&'static str>,
    failure_generation: u64,
    capture_generation: u64,
    capture_ready: bool,
    paste_error_code: Option<&'static str>,
}

impl OverlayNotice {
    pub const PASTE: u8 = 1;
    pub const PLAYBACK: u8 = 2;
    pub const CAPTURE_BUSY: u8 = 4;

    pub const fn new() -> Self {
        Self {
            failures: 0,
            generation: 0,
            progress: None,
            failure_generation: 0,
            capture_generation: 0,
            capture_ready: false,
            paste_error_code: None,
        }
    }

    pub fn report(&mut self, failure: u8) {
        if failure & Self::PASTE != 0 {
            self.paste_error_code = None;
        }
        self.failures |= failure;
        self.failure_generation += 1;
        self.shown();
    }

    pub fn report_paste_error(&mut self, code: &str) {
        self.report(Self::PASTE);
        self.paste_error_code = match code {
            "clipboard_restore_failed" => Some("clipboard_restore_failed"),
            "input_unconfirmed" => Some("input_unconfirmed"),
            "keys_held" => Some("keys_held"),
            "keyboard_unavailable" => Some("keyboard_unavailable"),
            _ => None,
        };
    }

    pub fn paste_error_code(&self) -> &'static str {
        self.paste_error_code.unwrap_or("failed")
    }

    pub fn failure_generation(&self) -> u64 {
        self.failure_generation
    }

    pub fn progress(&self) -> Option<&'static str> {
        self.progress
    }

    pub fn capture_generation(&self) -> u64 {
        self.capture_generation
    }

    pub fn capture_ready(&self) -> bool {
        self.capture_ready
    }

    pub fn set_progress(&mut self, state: &'static str) {
        self.progress = Some(state);
        if matches!(state, "recording" | "streaming") {
            self.capture_generation += 1;
            self.capture_ready = false;
        }
        self.shown();
    }

    pub fn recording_ready(&mut self) {
        if matches!(self.progress, Some("recording" | "streaming")) {
            self.capture_ready = true;
            self.shown();
        }
    }

    pub fn finish(&mut self) {
        self.progress = None;
        self.capture_ready = false;
        self.shown();
    }

    pub fn failures(&self) -> u8 {
        self.failures
    }

    pub fn shown(&mut self) {
        self.generation += 1;
    }

    pub fn hide_ticket(&self) -> Option<u64> {
        (self.failures == 0).then_some(self.generation)
    }

    pub fn can_hide(&self, ticket: u64) -> bool {
        self.failures == 0 && self.generation == ticket
    }

    pub fn dismiss(&mut self, failure_generation: u64) -> bool {
        if self.failure_generation != failure_generation {
            return false;
        }
        self.failures = 0;
        self.paste_error_code = None;
        self.shown();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_busy_is_visible_without_creating_or_finishing_capture() {
        let mut notice = OverlayNotice::new();
        notice.report(OverlayNotice::CAPTURE_BUSY);
        assert_eq!(notice.failures(), OverlayNotice::CAPTURE_BUSY);
        assert_eq!(notice.progress(), None);
        assert_eq!(notice.capture_generation(), 0);
        assert!(!notice.capture_ready());
        assert!(notice.hide_ticket().is_none());
        let old = notice.failure_generation();
        notice.report(OverlayNotice::CAPTURE_BUSY);
        assert!(!notice.dismiss(old));
        assert!(notice.dismiss(notice.failure_generation()));
        assert!(notice.hide_ticket().is_some());
        assert_eq!(notice.progress(), None);
    }

    #[test]
    fn paste_reason_survives_playback_notice_and_clears_only_on_current_dismissal() {
        let mut notice = OverlayNotice::new();
        notice.report_paste_error("clipboard_restore_failed");
        let old_generation = notice.failure_generation();
        assert_eq!(notice.paste_error_code(), "clipboard_restore_failed");
        notice.report(OverlayNotice::PLAYBACK);
        assert!(!notice.dismiss(old_generation));
        assert_eq!(notice.paste_error_code(), "clipboard_restore_failed");
        assert!(notice.dismiss(notice.failure_generation()));
        assert_eq!(notice.paste_error_code(), "failed");
    }

    #[test]
    fn paste_notice_never_exposes_arbitrary_error_payload() {
        let mut notice = OverlayNotice::new();
        notice.report_paste_error("synthetic arbitrary payload");
        assert_eq!(notice.paste_error_code(), "failed");
        notice.report_paste_error("input_unconfirmed");
        assert_eq!(notice.paste_error_code(), "input_unconfirmed");
        notice.report(OverlayNotice::PASTE);
        assert_eq!(notice.paste_error_code(), "failed");
    }

    #[test]
    fn normal_completion_and_new_progress_do_not_erase_unacknowledged_failure() {
        let mut notice = OverlayNotice::default();
        notice.report(OverlayNotice::PLAYBACK);
        notice.shown(); // Recognition/cleanup progresses after mute restoration failed.
        assert_eq!(notice.hide_ticket(), None);
        assert_eq!(notice.failures(), OverlayNotice::PLAYBACK);
    }

    #[test]
    fn pending_hide_cannot_hide_a_new_failure() {
        let mut notice = OverlayNotice::default();
        let ticket = notice.hide_ticket().unwrap();
        notice.report(OverlayNotice::PASTE);
        assert!(!notice.can_hide(ticket));
    }

    #[test]
    fn both_failures_remain_until_explicit_dismissal() {
        let mut notice = OverlayNotice::default();
        notice.report(OverlayNotice::PLAYBACK);
        notice.report(OverlayNotice::PASTE);
        assert_eq!(notice.failures(), 3);
        assert_eq!(notice.hide_ticket(), None);
        notice.dismiss(notice.failure_generation());
        assert_eq!(notice.failures(), 0);
        assert!(notice.can_hide(notice.hide_ticket().unwrap()));
    }

    #[test]
    fn dismiss_during_capture_preserves_the_ready_recording() {
        let mut notice = OverlayNotice::default();
        notice.set_progress("recording");
        notice.recording_ready();
        let capture = notice.capture_generation();
        notice.report(OverlayNotice::PLAYBACK);
        assert!(notice.dismiss(notice.failure_generation()));
        assert_eq!(notice.progress(), Some("recording"));
        assert!(notice.capture_ready());
        assert_eq!(notice.capture_generation(), capture);
    }

    #[test]
    fn dismiss_restores_the_latest_processing_stage() {
        let mut notice = OverlayNotice::default();
        notice.set_progress("recording");
        notice.report(OverlayNotice::PLAYBACK);
        let acknowledged = notice.failure_generation();
        notice.set_progress("transcribing");
        notice.set_progress("processing");
        assert_eq!(notice.failures(), OverlayNotice::PLAYBACK);
        assert!(notice.dismiss(acknowledged));
        assert_eq!(notice.progress(), Some("processing"));
    }

    #[test]
    fn completion_or_cancellation_does_not_resurrect_a_covered_stage() {
        let mut notice = OverlayNotice::default();
        notice.set_progress("processing");
        notice.report(OverlayNotice::PASTE);
        notice.finish();
        assert_eq!(notice.progress(), None);
        assert_eq!(notice.failures(), OverlayNotice::PASTE);
        assert!(notice.dismiss(notice.failure_generation()));
        assert_eq!(notice.progress(), None);
        assert!(notice.can_hide(notice.hide_ticket().unwrap()));
    }

    #[test]
    fn a_queued_dismissal_cannot_acknowledge_a_new_same_kind_failure() {
        let mut notice = OverlayNotice::default();
        notice.report(OverlayNotice::PLAYBACK);
        let acknowledged = notice.failure_generation();
        notice.report(OverlayNotice::PLAYBACK);
        assert!(!notice.dismiss(acknowledged));
        assert_eq!(notice.failures(), OverlayNotice::PLAYBACK);
        assert_eq!(notice.hide_ticket(), None);
    }

    #[test]
    fn a_queued_dismissal_cannot_acknowledge_a_new_different_failure() {
        let mut notice = OverlayNotice::default();
        notice.report(OverlayNotice::PLAYBACK);
        let acknowledged = notice.failure_generation();
        notice.report(OverlayNotice::PASTE);
        assert!(!notice.dismiss(acknowledged));
        assert_eq!(notice.failures(), 3);
    }

    #[test]
    fn new_capture_invalidates_a_pending_hide_before_main_thread_rendering() {
        let mut notice = OverlayNotice::default();
        notice.finish();
        let ticket = notice.hide_ticket().unwrap();
        notice.set_progress("recording");
        assert!(!notice.can_hide(ticket));
        assert_eq!(notice.progress(), Some("recording"));
    }

    #[test]
    fn only_a_new_capture_resets_readiness_and_session_identity() {
        let mut notice = OverlayNotice::default();
        notice.set_progress("streaming");
        notice.recording_ready();
        let first_capture = notice.capture_generation();
        notice.report(OverlayNotice::PLAYBACK);
        notice.dismiss(notice.failure_generation());
        assert!(notice.capture_ready());
        assert_eq!(notice.capture_generation(), first_capture);
        notice.finish();
        notice.set_progress("recording");
        assert!(!notice.capture_ready());
        assert_ne!(notice.capture_generation(), first_capture);
    }

    #[test]
    fn readiness_after_completion_does_not_revive_a_capture() {
        let mut notice = OverlayNotice::default();
        notice.set_progress("recording");
        notice.finish();
        notice.recording_ready();
        assert_eq!(notice.progress(), None);
        assert!(!notice.capture_ready());
    }
}
