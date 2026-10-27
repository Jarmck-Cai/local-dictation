//! Pure control flow for the protected paste transaction. No OS access.
use std::time::Duration;

pub(crate) const CANCELLED: &str = "manual_paste_cancelled";
pub(crate) const INJECTION_FAILED: &str = "manual_paste_injection_failed";
pub(crate) const RESTORE_FAILED: &str = "manual_paste_restore_failed";
pub(crate) const LIMIT_EXCEEDED: &str = "manual_paste_backup_too_large";
pub(crate) const MAX_FORMATS: usize = 64;
pub(crate) const MAX_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const READ_TIMEOUT: Duration = Duration::from_secs(3);
pub(crate) const READ_QUIET: Duration = Duration::from_millis(150);

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
                )
            ))
}

#[derive(Default)]
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
    pub ownership_lost: bool,
}
pub(crate) fn finished(elapsed: Duration, o: Observation) -> bool {
    o.ownership_lost
        || elapsed >= READ_TIMEOUT
        || (o.read && o.quiet_for.is_some_and(|quiet| quiet >= READ_QUIET))
}

pub(crate) trait Provider {
    fn prepare(&mut self, text: &str) -> Result<(), &'static str>;
    fn publish(&mut self) -> Result<(), &'static str>;
    fn begin_injection(&mut self) -> Result<(), &'static str>;
    fn observe(&mut self) -> Result<Observation, &'static str>;
    fn elapsed(&self) -> Duration;
    fn pause(&mut self);
    fn restore(&mut self) -> Result<(), &'static str>;
}

pub(crate) fn transact<P: Provider>(
    provider: &mut P,
    text: &str,
    inject: impl FnOnce() -> Result<(), String>,
    is_cancelled: impl Fn() -> bool,
) -> Result<bool, String> {
    if is_cancelled() {
        return Err(CANCELLED.into());
    }
    provider.prepare(text)?;
    if is_cancelled() {
        return Err(CANCELLED.into());
    }
    // Publish can fail after EmptyClipboard: even that path must settle.
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        provider.publish()?;
        if is_cancelled() {
            return Err(CANCELLED);
        }
        provider.begin_injection()?;
        if is_cancelled() {
            return Err(CANCELLED);
        }
        inject().map_err(|error| match error.as_str() {
            CANCELLED => CANCELLED,
            "keys_held" => "keys_held",
            "keyboard_unavailable" => "keyboard_unavailable",
            "unsupported" => "unsupported",
            _ => INJECTION_FAILED,
        })?;
        loop {
            if is_cancelled() {
                return Err(CANCELLED);
            }
            let observed = provider.observe()?;
            if finished(provider.elapsed(), observed) {
                return Ok(observed.read);
            }
            provider.pause();
        }
    }))
    .unwrap_or(Err(INJECTION_FAILED));
    // A restore failure must never be hidden by a read receipt or input error.
    provider.restore().map_err(String::from)?;
    outcome.map_err(String::from)
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
            if self.current == "temporary transcript" {
                self.current = self.original;
            }
            Ok(())
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
                ownership_lost: false
            }
        ));
        assert!(finished(
            Duration::from_secs(1),
            Observation {
                read: true,
                quiet_for: Some(Duration::from_millis(150)),
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
    fn unknown_or_handle_based_formats_refuse_backup_instead_of_being_skipped() {
        for format in [1, 7, 8, 13, 15, 16, 17] {
            assert!(safe_global_format(format, None));
        }
        for format in [2, 3, 9, 14, 0x80, 0x200, 0xc000] {
            assert!(!safe_global_format(format, None));
        }
        assert!(safe_global_format(0xc012, Some("HTML Format")));
        assert!(safe_global_format(0xc013, Some("Rich Text Format")));
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
}
