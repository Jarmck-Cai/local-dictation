//! Pure control flow for the protected paste transaction. No OS access.
use std::time::Duration;

pub(crate) const CANCELLED: &str = "manual_paste_cancelled";
pub(crate) const INJECTION_FAILED: &str = "manual_paste_injection_failed";
pub(crate) const RESTORE_FAILED: &str = "manual_paste_restore_failed";
pub(crate) const LIMIT_EXCEEDED: &str = "manual_paste_backup_too_large";
pub(crate) const PREPARE_BUSY: &str = "manual_paste_prepare_clipboard_busy";
pub(crate) const BEGIN_BUSY: &str = "manual_paste_begin_clipboard_busy";
pub(crate) const MAX_FORMATS: usize = 64;
pub(crate) const MAX_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const READ_TIMEOUT: Duration = Duration::from_secs(3);
pub(crate) const READ_QUIET: Duration = Duration::from_millis(150);
pub(crate) const PRE_RENDERED_QUIET: Duration = Duration::from_millis(480);
pub(crate) const OPEN_CLIPBOARD_TIMEOUT: Duration = Duration::from_millis(280);
pub(crate) const OPEN_BACKOFF_INITIAL: Duration = Duration::from_millis(2);
pub(crate) const OPEN_BACKOFF_MAX_STEP: Duration = Duration::from_millis(15);
pub(crate) const PREFLIGHT_REFRESH_AGE: Duration = Duration::from_millis(50);
pub(crate) const MAX_PREFLIGHT_ATTEMPTS: usize = 3;

pub(crate) fn open_retry_delay(
    step: Duration,
    elapsed: Duration,
    timeout: Duration,
) -> Option<(Duration, Duration)> {
    let remaining = timeout.checked_sub(elapsed)?;
    if remaining.is_zero() {
        return None;
    }
    let wait = step.min(OPEN_BACKOFF_MAX_STEP).min(remaining);
    let next_step = step.saturating_mul(2).min(OPEN_BACKOFF_MAX_STEP);
    Some((wait, next_step))
}

/// Valid only while the provider holds its clipboard lock.
pub(crate) fn same_lease(
    expected: u32,
    current: u32,
    same_owner: bool,
    ownership_lost: bool,
) -> bool {
    expected != 0 && current == expected && same_owner && !ownership_lost
}

/// Conservative reservation before CopyImage creates a DIB section.
pub(crate) fn bitmap_bytes(
    width: i32,
    height: i32,
    stride: i32,
    planes: u16,
    bits: u16,
) -> Result<usize, &'static str> {
    if width <= 0 || height == 0 || stride <= 0 || planes != 1 || bits == 0 || bits > 32 {
        return Err("manual_paste_backup_failed");
    }
    let row = (width as usize)
        .checked_mul(4)
        .ok_or(LIMIT_EXCEEDED)?
        .max(stride as usize);
    row.checked_mul(height.unsigned_abs() as usize)
        .and_then(|bytes| bytes.checked_add(4096))
        .ok_or(LIMIT_EXCEEDED)
}

pub(crate) fn safe_global_format(format: u32, name: Option<&str>) -> bool {
    matches!(format, 1 | 7 | 8 | 13 | 15 | 16 | 17)
        || (format >= 0xc000
            && matches!(
                name,
                Some(
                    "HTML Format"
                        | "Rich Text Format"
                        | "Rich Text Format Without Objects"
                        | "PNG"
                        | "image/png"
                        | "ExcludeClipboardContentFromMonitorProcessing"
                        | "CanIncludeInClipboardHistory"
                        | "CanUploadToCloudClipboard"
                        | "Clipboard Viewer Ignore"
                )
            ))
}

#[derive(Clone, Copy, Default)]
pub(crate) struct Budget {
    formats: usize,
    bytes: usize,
}
impl Budget {
    pub(crate) fn add(&mut self, bytes: usize) -> Result<(), &'static str> {
        let total = self.bytes.checked_add(bytes).ok_or(LIMIT_EXCEEDED)?;
        if bytes == 0 || total > MAX_BYTES || self.formats >= MAX_FORMATS {
            return Err(LIMIT_EXCEEDED);
        }
        self.formats += 1;
        self.bytes = total;
        Ok(())
    }
}

#[derive(Clone, Copy, Default)]
pub(crate) struct Observation {
    pub read: bool,
    pub quiet_for: Option<Duration>,
    pub pre_rendered_for: Option<Duration>,
    pub ownership_lost: bool,
}
pub(crate) fn finished(elapsed: Duration, o: Observation) -> bool {
    o.ownership_lost
        || elapsed >= READ_TIMEOUT
        || (o.read && o.quiet_for.is_some_and(|quiet| quiet >= READ_QUIET))
        || o.pre_rendered_for
            .is_some_and(|quiet| quiet >= PRE_RENDERED_QUIET)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Retention {
    Restore,
    CopyOnSuccess,
    CopyOnFailure,
}

impl Retention {
    pub(crate) const fn from_copy_flag(copy_result: bool) -> Self {
        if copy_result {
            Self::CopyOnSuccess
        } else {
            Self::Restore
        }
    }

    pub(crate) fn should_retain(self, outcome: &Result<bool, &'static str>) -> bool {
        match self {
            Self::Restore => false,
            Self::CopyOnSuccess => outcome.is_ok(),
            Self::CopyOnFailure => match *outcome {
                Ok(true) => false,
                Err(CANCELLED | "manual_paste_clipboard_changed") => false,
                Ok(false) | Err(_) => true,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SettlementAction {
    PreserveNewCopy,
    RestoreSnapshot,
    RetainResult,
}

/// The provider calls this with its lease and cancellation sample under the
/// final clipboard lock, immediately before the first settlement mutation.
pub(crate) fn settlement_action(
    copy_result: bool,
    same_lease: bool,
    cancelled: bool,
) -> SettlementAction {
    if !same_lease {
        SettlementAction::PreserveNewCopy
    } else if copy_result && !cancelled {
        SettlementAction::RetainResult
    } else {
        SettlementAction::RestoreSnapshot
    }
}

pub(crate) fn settlement_cancelled(is_cancelled: &dyn Fn() -> bool) -> bool {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(is_cancelled)).unwrap_or(true)
}

pub(crate) fn settlement_result(cancelled: bool) -> Result<(), &'static str> {
    if cancelled {
        Err(CANCELLED)
    } else {
        Ok(())
    }
}

pub(crate) trait Provider {
    fn prepare(&mut self, text: &str) -> Result<(), &'static str>;
    fn prepare_cancellable(
        &mut self,
        text: &str,
        _is_cancelled: &dyn Fn() -> bool,
    ) -> Result<(), &'static str> {
        self.prepare(text)
    }
    fn publish(&mut self) -> Result<(), &'static str>;
    /// Pump owner messages and absorb immediate post-close listener contention
    /// before minting a short-lived keyboard token.
    fn before_preflight(&mut self, _is_cancelled: &dyn Fn() -> bool) -> Result<(), &'static str> {
        Ok(())
    }
    fn begin_injection(&mut self) -> Result<(), &'static str>;
    fn begin_injection_cancellable(
        &mut self,
        _is_cancelled: &dyn Fn() -> bool,
    ) -> Result<(), &'static str> {
        self.begin_injection()
    }
    fn reset_injection_gate(&mut self) {}
    fn mark_injected(&mut self) {}
    fn observe(&mut self) -> Result<Observation, &'static str>;
    fn elapsed(&self) -> Duration;
    fn pause(&mut self);
    fn restore(&mut self) -> Result<(), &'static str>;
    /// Explicit copy preference, settled under the same ownership/sequence lock.
    fn retain_text(&mut self) -> Result<(), &'static str> {
        Err("automatic_paste_copy_unavailable")
    }
    /// Includes a lease change discovered while settling, after the last poll.
    fn ownership_lost(&self) -> bool {
        false
    }
    /// Locking providers override this and sample cancellation after obtaining
    /// their settlement lock, before choosing retention or restoring a snapshot.
    fn settle(
        &mut self,
        copy_result: bool,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<(), &'static str> {
        let cancelled = settlement_cancelled(is_cancelled);
        if copy_result && !cancelled {
            if let Err(error) = self.retain_text() {
                self.restore()?;
                return Err(error);
            }
            // Compatibility for non-locking providers: a cancellation raised
            // inside retention must not leave the result persistently copied.
            if settlement_cancelled(is_cancelled) {
                self.restore()?;
                return Err(CANCELLED);
            }
        } else {
            self.restore()?;
        }
        settlement_result(cancelled || settlement_cancelled(is_cancelled))
    }
}

pub(crate) fn transact<P: Provider>(
    provider: &mut P,
    text: &str,
    inject: impl FnOnce() -> Result<(), String>,
    is_cancelled: impl Fn() -> bool,
) -> Result<bool, String> {
    transact_with_retention(provider, text, inject, is_cancelled, false)
}

pub(crate) fn transact_with_retention<P: Provider>(
    provider: &mut P,
    text: &str,
    inject: impl FnOnce() -> Result<(), String>,
    is_cancelled: impl Fn() -> bool,
    copy_result: bool,
) -> Result<bool, String> {
    transact_with_preflight(provider, text, || Ok(()), inject, is_cancelled, copy_result)
}

pub(crate) fn transact_with_preflight<P: Provider>(
    provider: &mut P,
    text: &str,
    preflight: impl FnMut() -> Result<(), String>,
    inject: impl FnOnce() -> Result<(), String>,
    is_cancelled: impl Fn() -> bool,
    copy_result: bool,
) -> Result<bool, String> {
    transact_with_preflight_and_retention(
        provider,
        text,
        preflight,
        inject,
        is_cancelled,
        Retention::from_copy_flag(copy_result),
    )
}

pub(crate) fn transact_with_preflight_and_retention<P: Provider>(
    provider: &mut P,
    text: &str,
    mut preflight: impl FnMut() -> Result<(), String>,
    inject: impl FnOnce() -> Result<(), String>,
    is_cancelled: impl Fn() -> bool,
    retention: Retention,
) -> Result<bool, String> {
    if is_cancelled() {
        return Err(CANCELLED.into());
    }
    provider.prepare_cancellable(text, &is_cancelled)?;
    if is_cancelled() {
        return Err(CANCELLED.into());
    }
    // Publish can fail after EmptyClipboard: even that path must settle.
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        provider.publish()?;
        if is_cancelled() {
            return Err(CANCELLED);
        }
        provider.before_preflight(&is_cancelled)?;
        let mut attempts = 0usize;
        loop {
            if attempts > 0 && is_cancelled() {
                return Err(CANCELLED);
            }
            // A keyboard/source barrier can wait. It MUST precede the final lease
            // check so a new copy during that wait cannot authorize an old paste.
            preflight().map_err(|error| match error.as_str() {
                CANCELLED => CANCELLED,
                "keys_held" => "keys_held",
                _ => "keyboard_unavailable",
            })?;
            if is_cancelled() {
                return Err(CANCELLED);
            }
            let token_minted_at = provider.elapsed();
            provider.begin_injection_cancellable(&is_cancelled)?;
            if is_cancelled() {
                return Err(CANCELLED);
            }
            if provider.elapsed().saturating_sub(token_minted_at) < PREFLIGHT_REFRESH_AGE {
                break;
            }
            provider.reset_injection_gate();
            attempts += 1;
            if attempts >= MAX_PREFLIGHT_ATTEMPTS {
                return Err(BEGIN_BUSY);
            }
        }
        inject().map_err(|error| match error.as_str() {
            CANCELLED => CANCELLED,
            "keys_held" => "keys_held",
            "keyboard_unavailable" => "keyboard_unavailable",
            "unsupported" => "unsupported",
            _ => INJECTION_FAILED,
        })?;
        provider.mark_injected();
        loop {
            if is_cancelled() {
                return Err(CANCELLED);
            }
            let observed = provider.observe()?;
            if finished(provider.elapsed(), observed) {
                if is_cancelled() {
                    return Err(CANCELLED);
                }
                let delivered = observed.read
                    || observed
                        .pre_rendered_for
                        .is_some_and(|quiet| quiet >= PRE_RENDERED_QUIET);
                return Ok(delivered && !observed.ownership_lost);
            }
            provider.pause();
        }
    }))
    .unwrap_or(Err(INJECTION_FAILED));
    // A restore failure must never be hidden by a read receipt or input error.
    provider
        .settle(retention.should_retain(&outcome), &is_cancelled)
        .map_err(String::from)?;
    outcome
        .map(|read| read && !provider.ownership_lost())
        .map_err(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[derive(Default)]
    struct Fake {
        original: &'static str,
        current: &'static str,
        failure: Option<&'static str>,
        log: Vec<&'static str>,
        elapsed: Duration,
        observed: bool,
        concurrent_copy: bool,
        concurrent_copy_on_settle: bool,
        cancel_at_settlement: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    }
    impl Provider for Fake {
        fn prepare(&mut self, _: &str) -> Result<(), &'static str> {
            self.log.push("prepare");
            if self.failure == Some("backup") {
                return Err("manual_paste_backup_failed");
            }
            self.original = self.current;
            Ok(())
        }
        fn publish(&mut self) -> Result<(), &'static str> {
            self.log.push("publish");
            self.current = "temporary transcript";
            if self.failure == Some("publish") {
                Err("manual_paste_publish_failed")
            } else {
                Ok(())
            }
        }
        fn begin_injection(&mut self) -> Result<(), &'static str> {
            self.log.push("begin");
            if self.failure == Some("changed") {
                self.current = "new user copy";
                return Err("manual_paste_clipboard_changed");
            }
            Ok(())
        }
        fn observe(&mut self) -> Result<Observation, &'static str> {
            if self.concurrent_copy {
                self.current = "new user copy";
            }
            if self.failure == Some("render") {
                return Err("manual_paste_render_failed");
            }
            Ok(Observation {
                read: self.observed,
                quiet_for: self.observed.then_some(self.elapsed),
                pre_rendered_for: None,
                ownership_lost: self.concurrent_copy,
            })
        }
        fn elapsed(&self) -> Duration {
            self.elapsed
        }
        fn pause(&mut self) {
            self.elapsed += Duration::from_millis(50);
        }
        fn restore(&mut self) -> Result<(), &'static str> {
            self.log.push("restore");
            if self.failure == Some("restore") {
                return Err(RESTORE_FAILED);
            }
            if self.concurrent_copy_on_settle {
                self.current = "new user copy";
            } else if self.current == "temporary transcript" {
                self.current = self.original;
            }
            Ok(())
        }
        fn retain_text(&mut self) -> Result<(), &'static str> {
            self.log.push("retain");
            if self.failure == Some("retain") {
                return Err("automatic_paste_copy_unavailable");
            }
            if self.concurrent_copy || self.concurrent_copy_on_settle {
                self.current = "new user copy";
            }
            Ok(())
        }
        fn ownership_lost(&self) -> bool {
            self.concurrent_copy || self.concurrent_copy_on_settle
        }
        fn settle(
            &mut self,
            copy_result: bool,
            is_cancelled: &dyn Fn() -> bool,
        ) -> Result<(), &'static str> {
            // Represents acquiring the provider's final lock after a wait.
            if let Some(flag) = &self.cancel_at_settlement {
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            let cancelled = settlement_cancelled(is_cancelled);
            match settlement_action(copy_result, !self.ownership_lost(), cancelled) {
                SettlementAction::RetainResult => {
                    if let Err(error) = self.retain_text() {
                        self.restore()?;
                        return Err(error);
                    }
                }
                SettlementAction::RestoreSnapshot | SettlementAction::PreserveNewCopy => {
                    self.restore()?
                }
            }
            settlement_result(cancelled)
        }
    }
    fn clipboard() -> Fake {
        Fake {
            current: "original rich content",
            ..Default::default()
        }
    }

    #[test]
    fn failed_backup_never_publishes_or_injects() {
        let mut p = clipboard();
        p.failure = Some("backup");
        let calls = Cell::new(0);
        assert_eq!(
            transact(
                &mut p,
                "hello",
                || {
                    calls.set(calls.get() + 1);
                    Ok(())
                },
                || false
            ),
            Err("manual_paste_backup_failed".into())
        );
        assert_eq!(p.current, "original rich content");
        assert_eq!(p.log, ["prepare"]);
        assert_eq!(calls.get(), 0);
    }
    #[test]
    fn injected_once_and_original_restored_after_a_read() {
        let mut p = clipboard();
        p.observed = true;
        let calls = Cell::new(0);
        assert_eq!(
            transact(
                &mut p,
                "hello",
                || {
                    calls.set(calls.get() + 1);
                    Ok(())
                },
                || false
            ),
            Ok(true)
        );
        assert_eq!(calls.get(), 1);
        assert_eq!(p.current, "original rich content");
        assert!(p.elapsed >= READ_QUIET);
    }
    #[test]
    fn no_receipt_times_out_and_restores_without_retry() {
        let mut p = clipboard();
        let calls = Cell::new(0);
        assert_eq!(
            transact(
                &mut p,
                "hello",
                || {
                    calls.set(calls.get() + 1);
                    Ok(())
                },
                || false
            ),
            Ok(false)
        );
        assert_eq!(calls.get(), 1);
        assert_eq!(p.elapsed, Duration::from_secs(3));
        assert_eq!(p.current, "original rich content");
    }
    #[test]
    fn newer_user_copy_wins() {
        let mut p = clipboard();
        p.concurrent_copy = true;
        assert_eq!(transact(&mut p, "hello", || Ok(()), || false), Ok(false));
        assert_eq!(p.current, "new user copy");
    }
    #[test]
    fn newer_user_copy_after_a_receipt_is_not_observed_success() {
        let mut p = clipboard();
        p.observed = true;
        p.concurrent_copy = true;
        assert_eq!(transact(&mut p, "hello", || Ok(()), || false), Ok(false));
        assert_eq!(p.current, "new user copy");
    }
    #[test]
    fn copy_preference_retains_text_without_an_extra_clipboard_write() {
        let mut p = clipboard();
        p.observed = true;
        assert_eq!(
            transact_with_retention(&mut p, "hello", || Ok(()), || false, true),
            Ok(true)
        );
        assert_eq!(p.current, "temporary transcript");
        assert_eq!(p.log, ["prepare", "publish", "begin", "retain"]);
    }
    #[test]
    fn copy_preference_can_retain_an_unconfirmed_attempt_but_never_claims_a_read() {
        let mut p = clipboard();
        assert_eq!(
            transact_with_retention(&mut p, "hello", || Ok(()), || false, true),
            Ok(false)
        );
        assert_eq!(p.current, "temporary transcript");
    }
    #[test]
    fn ownership_loss_during_settlement_overrides_an_earlier_receipt() {
        for copy in [false, true] {
            let mut p = clipboard();
            p.observed = true;
            p.concurrent_copy_on_settle = true;
            assert_eq!(
                transact_with_retention(&mut p, "hello", || Ok(()), || false, copy),
                Ok(false)
            );
            assert_eq!(p.current, "new user copy");
        }
    }
    #[test]
    fn copy_preference_does_not_retain_cancelled_or_failed_output() {
        for code in [CANCELLED, INJECTION_FAILED] {
            let mut p = clipboard();
            assert_eq!(
                transact_with_retention(&mut p, "hello", || Err(code.into()), || false, true),
                Err(code.into())
            );
            assert_eq!(p.current, "original rich content");
            assert!(!p.log.contains(&"retain"));
        }
    }
    #[test]
    fn retain_failure_still_attempts_snapshot_restoration() {
        let mut p = clipboard();
        p.observed = true;
        p.failure = Some("retain");
        assert_eq!(
            transact_with_retention(&mut p, "hello", || Ok(()), || false, true),
            Err("automatic_paste_copy_unavailable".into())
        );
        assert_eq!(p.current, "original rich content");
        assert_eq!(p.log, ["prepare", "publish", "begin", "retain", "restore"]);
    }
    #[test]
    fn cancellation_during_provider_settlement_cannot_report_a_read_success() {
        struct LateCancel<'a> {
            cancelled: &'a Cell<bool>,
            retained: bool,
            restored: bool,
        }
        impl Provider for LateCancel<'_> {
            fn prepare(&mut self, _: &str) -> Result<(), &'static str> {
                Ok(())
            }
            fn publish(&mut self) -> Result<(), &'static str> {
                Ok(())
            }
            fn begin_injection(&mut self) -> Result<(), &'static str> {
                Ok(())
            }
            fn observe(&mut self) -> Result<Observation, &'static str> {
                Ok(Observation {
                    read: true,
                    quiet_for: Some(READ_QUIET),
                    pre_rendered_for: None,
                    ownership_lost: false,
                })
            }
            fn elapsed(&self) -> Duration {
                Duration::from_millis(200)
            }
            fn pause(&mut self) {
                panic!("a quiet receipt is already available");
            }
            fn restore(&mut self) -> Result<(), &'static str> {
                self.cancelled.set(true);
                self.restored = true;
                Ok(())
            }
            fn retain_text(&mut self) -> Result<(), &'static str> {
                self.cancelled.set(true);
                self.retained = true;
                Ok(())
            }
        }
        for copy in [false, true] {
            let cancelled = Cell::new(false);
            let mut p = LateCancel {
                cancelled: &cancelled,
                retained: false,
                restored: false,
            };
            assert_eq!(
                transact_with_retention(
                    &mut p,
                    "synthetic-current",
                    || Ok(()),
                    || cancelled.get(),
                    copy
                ),
                Err(CANCELLED.into())
            );
            assert!(
                p.restored,
                "a cancellation during retention must restore the snapshot"
            );
        }
    }
    #[test]
    fn lock_wait_cancellation_uses_the_real_settlement_decision_before_retention() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };
        for copy in [false, true] {
            let cancelled = Arc::new(AtomicBool::new(false));
            let mut p = clipboard();
            p.observed = true;
            p.cancel_at_settlement = Some(Arc::clone(&cancelled));
            assert_eq!(
                transact_with_retention(
                    &mut p,
                    "synthetic-current",
                    || Ok(()),
                    || cancelled.load(Ordering::SeqCst),
                    copy
                ),
                Err(CANCELLED.into())
            );
            assert_eq!(p.current, "original rich content");
            assert!(!p.log.contains(&"retain"));
        }
    }
    #[test]
    fn lock_wait_cancel_preserves_a_new_copy_and_does_not_hide_restore_failure() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };
        let cancelled = Arc::new(AtomicBool::new(false));
        let mut p = clipboard();
        p.observed = true;
        p.concurrent_copy_on_settle = true;
        p.cancel_at_settlement = Some(Arc::clone(&cancelled));
        assert_eq!(
            transact_with_retention(
                &mut p,
                "synthetic-current",
                || Ok(()),
                || cancelled.load(Ordering::SeqCst),
                true
            ),
            Err(CANCELLED.into())
        );
        assert_eq!(p.current, "new user copy");
        let mut p = clipboard();
        p.observed = true;
        p.failure = Some("restore");
        cancelled.store(false, Ordering::SeqCst);
        p.cancel_at_settlement = Some(Arc::clone(&cancelled));
        assert_eq!(
            transact_with_retention(
                &mut p,
                "synthetic-current",
                || Ok(()),
                || cancelled.load(Ordering::SeqCst),
                true
            ),
            Err(RESTORE_FAILED.into())
        );
    }
    #[test]
    fn cancellation_before_publish_preserves_original() {
        let mut p = clipboard();
        let checks = Cell::new(0);
        assert_eq!(
            transact(
                &mut p,
                "hello",
                || panic!("must not inject"),
                || {
                    checks.set(checks.get() + 1);
                    checks.get() >= 2
                }
            ),
            Err(CANCELLED.into())
        );
        assert_eq!(p.log, ["prepare"]);
        assert_eq!(p.current, "original rich content");
    }
    #[test]
    fn cancellation_after_publish_still_restores() {
        let mut p = clipboard();
        let checks = Cell::new(0);
        assert_eq!(
            transact(
                &mut p,
                "hello",
                || panic!("must not inject"),
                || {
                    checks.set(checks.get() + 1);
                    checks.get() >= 3
                }
            ),
            Err(CANCELLED.into())
        );
        assert_eq!(p.current, "original rich content");
        assert_eq!(p.log, ["prepare", "publish", "restore"]);
    }
    #[test]
    fn injection_error_is_not_retried_and_still_restores() {
        let mut p = clipboard();
        assert_eq!(
            transact(
                &mut p,
                "hello",
                || Err("private OS detail".into()),
                || false
            ),
            Err(INJECTION_FAILED.into())
        );
        assert_eq!(p.current, "original rich content");
        assert_eq!(p.log, ["prepare", "publish", "begin", "restore"]);
    }
    #[test]
    fn injection_cancellation_preserves_category_after_restoring_clipboard() {
        let mut p = clipboard();
        assert_eq!(
            transact(&mut p, "hello", || Err(CANCELLED.into()), || false),
            Err(CANCELLED.into())
        );
        assert_eq!(p.current, "original rich content");
        assert_eq!(p.log, ["prepare", "publish", "begin", "restore"]);
    }
    #[test]
    fn injection_panic_still_restores() {
        let mut p = clipboard();
        assert_eq!(
            transact(
                &mut p,
                "hello",
                || panic!("simulated injection panic"),
                || false
            ),
            Err(INJECTION_FAILED.into())
        );
        assert_eq!(p.current, "original rich content");
    }
    #[test]
    fn partial_publication_failure_restores_without_injecting() {
        let mut p = clipboard();
        p.failure = Some("publish");
        assert_eq!(
            transact(&mut p, "hello", || panic!("must not inject"), || false),
            Err("manual_paste_publish_failed".into())
        );
        assert_eq!(p.current, "original rich content");
    }
    #[test]
    fn render_failure_is_visible_after_restoration() {
        let mut p = clipboard();
        p.failure = Some("render");
        assert_eq!(
            transact(&mut p, "hello", || Ok(()), || false),
            Err("manual_paste_render_failed".into())
        );
        assert_eq!(p.current, "original rich content");
    }
    #[test]
    fn restoration_error_overrides_apparent_success() {
        let mut p = clipboard();
        p.failure = Some("restore");
        p.observed = true;
        assert_eq!(
            transact(&mut p, "hello", || Ok(()), || false),
            Err(RESTORE_FAILED.into())
        );
    }
    #[test]
    fn budget_rejects_large_cumulative_snapshots_and_too_many_formats() {
        let mut b = Budget::default();
        b.add(40 * 1024 * 1024).unwrap();
        assert_eq!(b.add(30 * 1024 * 1024), Err(LIMIT_EXCEEDED));
        let mut b = Budget::default();
        for _ in 0..64 {
            b.add(1).unwrap();
        }
        assert_eq!(b.add(1), Err(LIMIT_EXCEEDED));
        assert_eq!(Budget::default().add(usize::MAX), Err(LIMIT_EXCEEDED));
    }
    #[test]
    fn read_quiet_period_and_deadline_are_bounded() {
        assert!(!finished(
            Duration::from_secs(1),
            Observation {
                read: true,
                quiet_for: Some(Duration::from_millis(149)),
                pre_rendered_for: None,
                ownership_lost: false
            }
        ));
        assert!(finished(
            Duration::from_secs(1),
            Observation {
                read: true,
                quiet_for: Some(Duration::from_millis(150)),
                pre_rendered_for: None,
                ownership_lost: false
            }
        ));
        assert!(!finished(
            Duration::from_millis(479),
            Observation {
                read: false,
                quiet_for: None,
                pre_rendered_for: Some(Duration::from_millis(479)),
                ownership_lost: false
            }
        ));
        assert!(finished(
            Duration::from_millis(480),
            Observation {
                read: false,
                quiet_for: None,
                pre_rendered_for: Some(PRE_RENDERED_QUIET),
                ownership_lost: false
            }
        ));
        assert!(finished(Duration::from_secs(3), Observation::default()));
        assert!(finished(
            Duration::ZERO,
            Observation {
                ownership_lost: true,
                ..Default::default()
            }
        ));
    }
    #[test]
    fn clipboard_lease_requires_nonzero_sequence_and_owner_under_lock() {
        assert!(same_lease(41, 41, true, false));
        assert!(!same_lease(41, 42, true, false));
        assert!(!same_lease(41, 41, false, false));
        assert!(!same_lease(0, 0, true, false));
        assert!(!same_lease(41, 41, true, true));
        // Provider records the sequence returned by its own successful render.
        assert!(same_lease(42, 42, true, false));
    }
    #[test]
    fn global_clone_allowlist_never_treats_unknown_or_gdi_handles_as_bytes() {
        for format in [1, 7, 8, 13, 15, 16, 17] {
            assert!(safe_global_format(format, None));
        }
        for format in [2, 3, 9, 14, 0x80, 0x200, 0xc000] {
            assert!(!safe_global_format(format, None));
        }
        assert!(safe_global_format(0xc012, Some("HTML Format")));
        assert!(safe_global_format(0xc013, Some("Rich Text Format")));
        assert!(safe_global_format(0xc015, Some("Clipboard Viewer Ignore")));
        assert!(!safe_global_format(
            0xc014,
            Some("Unknown application private object")
        ));
        assert!(!safe_global_format(0x200, Some("HTML Format")));
    }

    #[test]
    fn changed_clipboard_before_injection_does_not_send_a_paste() {
        let mut p = clipboard();
        p.failure = Some("changed");
        let calls = Cell::new(0);
        assert_eq!(
            transact(
                &mut p,
                "hello",
                || {
                    calls.set(calls.get() + 1);
                    Ok(())
                },
                || false
            ),
            Err("manual_paste_clipboard_changed".into())
        );
        assert_eq!(calls.get(), 0);
        assert_eq!(p.current, "new user copy");
    }
    #[test]
    fn cancellation_callback_panic_after_publication_still_restores() {
        let mut p = clipboard();
        let checks = Cell::new(0);
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            transact(
                &mut p,
                "hello",
                || Ok(()),
                || {
                    checks.set(checks.get() + 1);
                    if checks.get() >= 3 {
                        panic!("simulated cancellation callback panic");
                    }
                    false
                },
            )
        }));
        assert!(outcome.is_ok());
        assert_eq!(p.current, "original rich content");
    }
    #[test]
    fn bitmap_reservation_charges_worst_case_dib_and_refuses_unsupported_geometry() {
        assert_eq!(bitmap_bytes(16, 8, 4, 1, 1), Ok(4608));
        assert_eq!(bitmap_bytes(16, -8, 80, 1, 32), Ok(4736));
        assert!(bitmap_bytes(-1, 8, 4, 1, 1).is_err());
        assert!(bitmap_bytes(16, 0, 4, 1, 1).is_err());
        assert!(bitmap_bytes(16, 8, 4, 2, 1).is_err());
        assert!(bitmap_bytes(16, 8, 128, 1, 64).is_err());
        let mut budget = Budget::default();
        let bytes = bitmap_bytes(10000, 10000, 1252, 1, 1).unwrap();
        assert_eq!(budget.add(bytes), Err(LIMIT_EXCEEDED));
    }
    #[test]
    fn final_input_gate_rejections_are_preserved_after_restoration() {
        for code in ["keys_held", "keyboard_unavailable", "unsupported"] {
            let mut p = clipboard();
            assert_eq!(
                transact(&mut p, "hello", || Err(code.into()), || false),
                Err(code.into())
            );
            assert_eq!(p.current, "original rich content");
            assert_eq!(p.log, ["prepare", "publish", "begin", "restore"]);
        }
    }
    #[test]
    fn open_retry_delay_follows_exponential_backoff_and_caps_at_budget() {
        let mut step = OPEN_BACKOFF_INITIAL;
        let mut elapsed = Duration::ZERO;
        let mut waits = Vec::new();
        while let Some((wait, next)) = open_retry_delay(step, elapsed, OPEN_CLIPBOARD_TIMEOUT) {
            waits.push(wait.as_millis() as u64);
            elapsed += wait;
            step = next;
        }
        assert_eq!(&waits[..4], &[2, 4, 8, 15]);
        assert!(waits.iter().all(|&ms| ms > 0 && ms <= 15));
        assert_eq!(elapsed, OPEN_CLIPBOARD_TIMEOUT);
        assert!(open_retry_delay(step, elapsed, OPEN_CLIPBOARD_TIMEOUT).is_none());
        assert_ne!(PREPARE_BUSY, BEGIN_BUSY);
    }
    #[test]
    fn lock_wait_past_token_refresh_age_refreshes_preflight_and_rechecks_lease() {
        struct ContendedBegin {
            now: Duration,
            begin_waits: Vec<Duration>,
            resets: usize,
            copy_on_second_preflight: bool,
            ownership_lost: bool,
            restored: bool,
        }
        impl Provider for ContendedBegin {
            fn prepare(&mut self, _: &str) -> Result<(), &'static str> {
                Ok(())
            }
            fn publish(&mut self) -> Result<(), &'static str> {
                Ok(())
            }
            fn begin_injection(&mut self) -> Result<(), &'static str> {
                if let Some(wait) = self.begin_waits.first().copied() {
                    self.begin_waits.remove(0);
                    self.now += wait;
                }
                if self.ownership_lost {
                    Err("manual_paste_clipboard_changed")
                } else {
                    Ok(())
                }
            }
            fn reset_injection_gate(&mut self) {
                self.resets += 1;
                if self.copy_on_second_preflight {
                    self.ownership_lost = true;
                }
            }
            fn observe(&mut self) -> Result<Observation, &'static str> {
                Ok(Observation {
                    read: true,
                    quiet_for: Some(READ_QUIET),
                    pre_rendered_for: None,
                    ownership_lost: self.ownership_lost,
                })
            }
            fn elapsed(&self) -> Duration {
                self.now
            }
            fn pause(&mut self) {
                self.now += Duration::from_millis(10);
            }
            fn restore(&mut self) -> Result<(), &'static str> {
                self.restored = true;
                Ok(())
            }
            fn ownership_lost(&self) -> bool {
                self.ownership_lost
            }
        }
        let clock = std::rc::Rc::new(Cell::new(0u64));
        let token_at = std::rc::Rc::new(Cell::new(0u64));
        let preflights = Cell::new(0usize);
        let mut p = ContendedBegin {
            now: Duration::ZERO,
            begin_waits: vec![Duration::from_millis(120), Duration::ZERO],
            resets: 0,
            copy_on_second_preflight: false,
            ownership_lost: false,
            restored: false,
        };
        let clock_for_preflight = std::rc::Rc::clone(&clock);
        let token_for_preflight = std::rc::Rc::clone(&token_at);
        let result = transact_with_preflight(
            &mut p,
            "hello",
            || {
                preflights.set(preflights.get() + 1);
                // First preflight at 0ms, refreshed at 120ms after the slow lock wait.
                let now_ms = if preflights.get() == 1 { 0 } else { 120 };
                clock_for_preflight.set(now_ms);
                token_for_preflight.set(now_ms);
                Ok(())
            },
            || {
                // Enforce the real 100ms PreparedKeyboard token lifetime.
                if 120u64.saturating_sub(token_at.get()) > 100 {
                    Err("keys_held".into())
                } else {
                    Ok(())
                }
            },
            || false,
            false,
        );
        assert_eq!(result, Ok(true));
        assert_eq!(preflights.get(), 2);
        assert_eq!(p.resets, 1);
        assert!(p.restored);

        // A user copy during the refreshed preflight is caught by the re-run lease check.
        let mut p2 = ContendedBegin {
            now: Duration::ZERO,
            begin_waits: vec![Duration::from_millis(120), Duration::ZERO],
            resets: 0,
            copy_on_second_preflight: true,
            ownership_lost: false,
            restored: false,
        };
        assert_eq!(
            transact_with_preflight(
                &mut p2,
                "hello",
                || Ok(()),
                || panic!("must not inject over a new copy"),
                || false,
                false,
            ),
            Err("manual_paste_clipboard_changed".into())
        );
    }
    #[test]
    fn copy_on_failure_restores_on_success_retains_on_failure_and_protects_cancel_or_new_copy() {
        // 1. Confirmed read restores original clipboard just like dont_modify.
        let mut ok_paste = Fake {
            current: "original rich content",
            observed: true,
            elapsed: READ_QUIET,
            ..Default::default()
        };
        assert_eq!(
            transact_with_preflight_and_retention(
                &mut ok_paste,
                "hello",
                || Ok(()),
                || Ok(()),
                || false,
                Retention::CopyOnFailure,
            ),
            Ok(true)
        );
        assert_eq!(ok_paste.current, "original rich content");
        assert_eq!(ok_paste.log, ["prepare", "publish", "begin", "restore"]);

        // 2. Key/injection failure retains transcript on the clipboard.
        for code in ["keys_held", "keyboard_unavailable", INJECTION_FAILED] {
            let mut failed_paste = Fake {
                current: "original rich content",
                ..Default::default()
            };
            assert_eq!(
                transact_with_preflight_and_retention(
                    &mut failed_paste,
                    "hello",
                    || Ok(()),
                    || Err(code.into()),
                    || false,
                    Retention::CopyOnFailure,
                ),
                Err(code.into())
            );
            assert_eq!(failed_paste.current, "temporary transcript");
            assert_eq!(failed_paste.log, ["prepare", "publish", "begin", "retain"]);
        }

        // 3. Unconfirmed 3s timeout (no read, no concurrent copy) retains transcript.
        let mut unconfirmed = Fake {
            current: "original rich content",
            observed: false,
            ..Default::default()
        };
        assert_eq!(
            transact_with_preflight_and_retention(
                &mut unconfirmed,
                "hello",
                || Ok(()),
                || Ok(()),
                || false,
                Retention::CopyOnFailure,
            ),
            Ok(false)
        );
        assert_eq!(unconfirmed.current, "temporary transcript");
        assert_eq!(unconfirmed.log, ["prepare", "publish", "begin", "retain"]);

        // 4. Esc cancellation never overwrites original clipboard.
        let cancelled = Cell::new(false);
        let mut cancelled_paste = Fake {
            current: "original rich content",
            ..Default::default()
        };
        assert_eq!(
            transact_with_preflight_and_retention(
                &mut cancelled_paste,
                "hello",
                || {
                    cancelled.set(true);
                    Err(CANCELLED.into())
                },
                || panic!("must not inject after cancel"),
                || cancelled.get(),
                Retention::CopyOnFailure,
            ),
            Err(CANCELLED.into())
        );
        assert_eq!(cancelled_paste.current, "original rich content");
        assert_eq!(cancelled_paste.log, ["prepare", "publish", "restore"]);

        // 5. Concurrent user Ctrl+C before or after injection never gets overwritten.
        let mut changed_before = Fake {
            current: "original rich content",
            failure: Some("changed"),
            ..Default::default()
        };
        assert_eq!(
            transact_with_preflight_and_retention(
                &mut changed_before,
                "hello",
                || Ok(()),
                || panic!("must not inject"),
                || false,
                Retention::CopyOnFailure,
            ),
            Err("manual_paste_clipboard_changed".into())
        );
        assert_eq!(changed_before.current, "new user copy");

        let mut changed_during_observe = Fake {
            current: "original rich content",
            concurrent_copy: true,
            ..Default::default()
        };
        assert_eq!(
            transact_with_preflight_and_retention(
                &mut changed_during_observe,
                "hello",
                || Ok(()),
                || Ok(()),
                || false,
                Retention::CopyOnFailure,
            ),
            Ok(false)
        );
        assert_eq!(changed_during_observe.current, "new user copy");
    }
}
