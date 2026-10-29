"""Check actual WinClipboard publication methods against synthetic Win32 contracts.

No native clipboard, keyboard, window, model, audio, or personal data APIs are
linked or executed. This verifies the adapter's publication/lease/settlement
contract, not that a real target inserts text. --baseline reads v0.1.7 and is
expected to fail the same assertions, preserving the original red regression.
--rdp additionally characterizes selected clipboard-redirection schedules using
the same production methods and the production one-shot chord controller. These
are possible schedules, not observations of Microsoft's RDP implementation.
--rdp-delivery-gate adds delivery assertions for the known failing schedules;
with the current algorithm this command MUST fail. A green characterization
suite must never be confused with those hypothetical delivery gaps being fixed.

The CloseClipboard synthesis model follows Wine's Windows compatibility tests:
https://github.com/wine-mirror/wine/blob/master/dlls/user32/tests/clipboard.c
In test_messages/clipboard_thread, closing after text publication adds synthesized
formats and advances the sequence; closing without synthesis does not.
"""

from pathlib import Path
import argparse
import hashlib
import importlib.util
import json
import os
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("automatic_paste_check", ROOT / "scripts/check-automatic-paste.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
function = module.function


def declaration(source: str, name: str) -> str:
    """Extract a complete production struct, without recreating its fields."""
    start = source.index(f"struct {name} {{")
    opening = source.index("{", start)
    depth, end = 1, opening + 1
    while depth:
        depth += (source[end] == "{") - (source[end] == "}")
        end += 1
    return source[start:end]


ADAPTERS = r'''
#![allow(dead_code, non_snake_case)]
use std::cell::{Cell, RefCell};
use std::time::Duration;
use policy::{Observation, Provider, RESTORE_FAILED};
const UNICODE_TEXT: u32 = 13;
const ERROR_SUCCESS: u32 = 0;
const UNAVAILABLE: &str = "manual_paste_unavailable";
const PUBLISH_FAILED: &str = "manual_paste_publish_failed";
const RENDER_FAILED: &str = "manual_paste_render_failed";
const BACKUP_FAILED: &str = "manual_paste_backup_failed";
const OPEN_RESTORE_TIMEOUT: Duration = Duration::from_millis(500);
const POLL: Duration = Duration::from_millis(10);
#[derive(Clone, Copy, Debug, PartialEq, Eq)] struct HWND(usize);
#[derive(Clone, Copy, Debug)] struct Instant(u64);
impl Instant {
    fn now() -> Self { Self(SYSTEM.with(|s| s.borrow().now)) }
    fn elapsed(self) -> Duration { Duration::from_millis(SYSTEM.with(|s| s.borrow().now) - self.0) }
}
mod thread {
    pub fn sleep(duration: std::time::Duration) {
        super::SYSTEM.with(|s| s.borrow_mut().now += duration.as_millis() as u64);
    }
}
#[derive(Default)]
struct SyntheticSystem {
    now: u64, sequence: u32, owner: usize, opened: Option<usize>,
    formats: Vec<u32>, body: &'static str, delayed: bool, synthesized: bool,
    close_synthesis: bool, context: usize, rendering: bool,
    copy_on_publish_close: bool, cancel_on_publish_close: bool,
    copy_on_readonly_close: bool, copy_on_copy_open: bool, fail_marker: bool,
    empty_calls: usize, fail_first_empty: bool,
    suppress_owner_notification: bool,
    copy_before_begin: bool, change_before_begin: bool,
    cancelled: bool, reader_pending: bool, copy_after_injection: bool,
    cancel_after_injection: bool, injected: usize, restored: usize,
    restore_unavailable: bool, restore_failure: bool, originally_empty: bool, marker_only: bool,
    partial_restore: bool,
    fail_retention: bool, cancel_on_settle: bool, fail_text_allocation: bool, text_allocations: usize,
    failed_payload_with_markers: bool,
    rdp: bool, remote_formats_at: Option<u64>, remote_formats_ready: bool,
    remote_request_at: Option<u64>, remote_request_delay: u64,
    remote_consume_at: Option<u64>, remote_consume_delay: u64,
    remote_buffer: Option<&'static str>, remote_result: Option<&'static str>,
    remote_requests: usize, prefetch_before_begin: bool,
    busy_before_begin_until: Option<u64>, copy_at: Option<u64>, cancel_at: Option<u64>,
}
thread_local! { static SYSTEM: RefCell<SyntheticSystem> = RefCell::new(SyntheticSystem::default()); }
fn reset(synthesis: bool) {
    SYSTEM.with(|s| *s.borrow_mut() = SyntheticSystem {
        sequence: 10, owner: 9, formats: vec![13], body: "synthetic-original",
        close_synthesis: synthesis, ..Default::default()
    });
}
fn new_user_copy(owner_changed: bool) {
    let (context, suppress_notification) = SYSTEM.with(|s| {
        let mut s = s.borrow_mut();
        s.sequence += 10;
        if owner_changed { s.owner = 2; }
        s.body = "synthetic-new-user-copy";
        s.formats = vec![13]; s.delayed = false; s.synthesized = true;
        (s.context, s.suppress_owner_notification)
    });
    // Fake delivery of the OS ownership-loss notification, without a window.
    if owner_changed && context != 0 && !suppress_notification {
        unsafe { (&*(context as *const Context)).ownership_lost.set(true); }
    }
}
unsafe fn GetClipboardSequenceNumber() -> u32 { SYSTEM.with(|s| s.borrow().sequence) }
unsafe fn GetClipboardOwner() -> Result<HWND, ()> { Ok(HWND(SYSTEM.with(|s| s.borrow().owner))) }
unsafe fn SetLastError(_: u32) {}
unsafe fn GetLastError() -> u32 { ERROR_SUCCESS }
unsafe fn SetClipboardData(format: u32, handle: Option<usize>) -> Result<usize, ()> {
    SYSTEM.with(|s| {
        let mut s = s.borrow_mut();
        if format == 50002 && s.fail_marker { return Err(()); }
        if format == 13 && handle.is_some() && !s.rendering && s.fail_retention { return Err(()); }
        if !s.formats.contains(&format) { s.formats.push(format); }
        // Rendering callbacks fulfill an existing promise. Wine's Windows tests
        // also check that SetClipboardData inside WM_RENDERFORMAT keeps sequence.
        if !s.rendering { s.sequence += 1; }
        if format == UNICODE_TEXT {
            s.delayed = handle.is_none();
            s.body = if handle.is_some() { "synthetic-transcript" } else { "unrendered" };
        }
        Ok(handle.unwrap_or(0))
    })
}
unsafe fn EmptyClipboard() -> Result<(), ()> {
    SYSTEM.with(|s| {
        let mut s = s.borrow_mut(); s.empty_calls += 1;
        if s.fail_first_empty && s.empty_calls == 1 { return Err(()); }
        s.owner = s.opened.expect("clipboard lock required");
        s.formats.clear(); s.sequence += 1; s.body = "empty";
        s.delayed = false; s.synthesized = false;
        Ok(())
    })
}
struct Global;
impl Global {
    fn from_bytes(_: &[u8]) -> Result<Self, &'static str> { Ok(Self) }
    fn transfer(self, format: u32) -> Result<(), &'static str> {
        unsafe { SetClipboardData(format, Some(1)) }.map(|_| ()).map_err(|_| PUBLISH_FAILED)
    }
}
struct Saved { format: u32 }
impl Saved {
    // Allocation/snapshot restoration is an API boundary for this harness.
    // Snapshot byte ownership has its separate check-win32-snapshot.py suite.
    fn restore(&mut self) -> Result<(), &'static str> {
        SYSTEM.with(|s| {
            let mut s = s.borrow_mut(); s.restored += 1;
            if s.restore_failure && self.format == 13 { return Err(RESTORE_FAILED); }
            s.body = "synthetic-original"; s.delayed = false;
            if !s.formats.contains(&self.format) { s.formats.push(self.format); }
            Ok(())
        })
    }
}
struct PCWSTR(*const u16);
unsafe fn RegisterClipboardFormatW(name: PCWSTR) -> u32 {
    // Synthetic identity only; no native clipboard API is linked.
    if *name.0 == 'E' as u16 { 50001 } else if *name.0.add(3) == 'I' as u16 { 50002 } else { 50003 }
}
fn text_memory(text: &str) -> Result<Global, &'static str> {
    assert_eq!(text, "synthetic-transcript");
    SYSTEM.with(|s| { let mut s = s.borrow_mut(); s.text_allocations += 1;
        if s.fail_text_allocation && s.text_allocations == 2 { Err(BACKUP_FAILED) } else { Ok(Global) } })
}
fn enumerate() -> Result<Vec<u32>, &'static str> {
    SYSTEM.with(|s| { let s = s.borrow();
        Ok(if s.originally_empty { vec![] } else if s.restore_unavailable { vec![60001] }
           else if s.marker_only { vec![16, 50001, 50002, 50003] } else { vec![13] }) })
}
fn snapshot() -> Result<Vec<Saved>, &'static str> {
    SYSTEM.with(|s| { let s = s.borrow();
        Ok(if s.originally_empty || s.restore_unavailable { vec![] }
           else if s.marker_only { vec![Saved { format: 16 }, Saved { format: 50001 }] }
           else if s.partial_restore { vec![Saved { format: 13 }, Saved { format: 50010 }] }
           else if s.failed_payload_with_markers { vec![Saved { format: 13 }, Saved { format: 16 }, Saved { format: 50001 }] }
           else { vec![Saved { format: 13 }] }) })
}
struct ClipboardLock;
impl ClipboardLock {
    fn open(hwnd: HWND) -> Result<Self, &'static str> {
        if SYSTEM.with(|s| std::mem::take(&mut s.borrow_mut().copy_on_copy_open)) {
            new_user_copy(true);
        }
        SYSTEM.with(|s| {
            let mut s = s.borrow_mut();
            if s.opened.is_some() { return Err("manual_paste_clipboard_busy"); }
            if s.rdp && s.synthesized && s.injected == 0
                && s.busy_before_begin_until.is_some_and(|until| s.now < until) {
                return Err("manual_paste_clipboard_busy");
            }
            if s.injected > 0 && s.cancel_on_settle { s.cancelled = true; }
            s.opened = Some(hwnd.0); Ok(Self)
        })
    }
    fn close(self) -> Result<(), &'static str> {
        let (copy, cancel) = SYSTEM.with(|s| {
            let mut s = s.borrow_mut();
            let publication = s.delayed && !s.synthesized;
            if publication {
                if s.close_synthesis { s.sequence += 2; s.formats.extend([16, 1, 7]); }
                s.synthesized = true;
            }
            s.opened = None;
            (std::mem::take(&mut s.copy_on_readonly_close) || (publication && std::mem::take(&mut s.copy_on_publish_close)),
             publication && std::mem::take(&mut s.cancel_on_publish_close))
        });
        if copy { new_user_copy(true); }
        if cancel { SYSTEM.with(|s| s.borrow_mut().cancelled = true); }
        Ok(())
    }
}
struct Window { hwnd: HWND }
impl Window {
    fn create(context: &Context) -> Result<Self, &'static str> {
        SYSTEM.with(|s| s.borrow_mut().context = context as *const Context as usize);
        Ok(Self { hwnd: HWND(1) })
    }
    fn pump(&self) {
        redirect_tick(self.hwnd);
        let (owner_change, same_owner_change, read, context) = SYSTEM.with(|s| {
            let mut s = s.borrow_mut();
            (std::mem::take(&mut s.copy_before_begin),
             std::mem::take(&mut s.change_before_begin),
             std::mem::take(&mut s.reader_pending), s.context)
        });
        if owner_change { new_user_copy(true); }
        if same_owner_change { new_user_copy(false); }
        if read && context != 0 {
            SYSTEM.with(|s| s.borrow_mut().rendering = true);
            unsafe { render(self.hwnd, &*(context as *const Context), true); }
            SYSTEM.with(|s| s.borrow_mut().rendering = false);
        }
    }
}
impl Drop for Window {
    fn drop(&mut self) {
        SYSTEM.with(|s| s.borrow_mut().context = 0);
    }
}

// Only enabled by --rdp cases. Format advertisement, requesting bytes and the
// remote editor consuming the response are intentionally separate events.
fn redirect_tick(hwnd: HWND) {
    let (copy, read, context) = SYSTEM.with(|s| {
        let mut s = s.borrow_mut();
        if s.cancel_at.is_some_and(|at| at <= s.now) { s.cancelled = true; }
        let copy = s.copy_at.is_some_and(|at| at <= s.now);
        if copy { s.copy_at = None; }
        if s.remote_formats_at.is_some_and(|at| at <= s.now) {
            s.remote_formats_ready = true; s.remote_formats_at = None;
        }
        let read = s.remote_request_at.is_some_and(|at| at <= s.now);
        if read { s.remote_request_at = None; s.remote_requests += 1; }
        let prefetch = std::mem::take(&mut s.prefetch_before_begin);
        (copy, read || prefetch, s.context)
    });
    if copy { new_user_copy(true); }
    if read && context != 0 {
        SYSTEM.with(|s| s.borrow_mut().rendering = true);
        unsafe { render(hwnd, &*(context as *const Context), true); }
        SYSTEM.with(|s| s.borrow_mut().rendering = false);
    }
    SYSTEM.with(|s| {
        let mut s = s.borrow_mut();
        // The synthetic endpoint copies bytes into its response before letting
        // the remote editor consume them. No claim about real network latency.
        if s.remote_requests > 0 && s.remote_buffer.is_none() {
            s.remote_buffer = Some(s.body);
            s.remote_consume_at = Some(s.now + s.remote_consume_delay);
        }
        if s.remote_consume_at.is_some_and(|at| at <= s.now) {
            s.remote_result = s.remote_buffer; s.remote_consume_at = None;
        }
    });
}
'''

PREPARE = r'''
    fn prepare(&mut self, _: &str) -> Result<(), &'static str> {
        *self.context.text.borrow_mut() = Some(Global);
        if self.copy_result { self.retained_text = Some(Global); }
        self.saved = vec![Saved];
        if SYSTEM.with(|s| s.borrow().restore_unavailable || s.borrow().originally_empty) {
            self.saved.clear();
        }
        self.markers = vec![(50001, Global), (50002, Global), (50003, Global)];
        self.held = Some(ClipboardLock::open(self.window.hwnd)?);
        Ok(())
    }
'''

TESTS = r'''
#[cfg(test)]
mod publication_tests {
    use super::*;
    fn transaction(copy_result: bool) -> Result<bool, String> {
        let mut clipboard = WinClipboard::new(copy_result).unwrap();
        policy::transact_with_retention(&mut clipboard, "synthetic-transcript", || {
            let (copy, cancel) = SYSTEM.with(|s| {
                let mut s = s.borrow_mut(); s.injected += 1;
                s.reader_pending = true;
                (std::mem::take(&mut s.copy_after_injection), s.cancel_after_injection)
            });
            if copy { new_user_copy(true); }
            if cancel { SYSTEM.with(|s| s.borrow_mut().cancelled = true); }
            Ok(())
        }, || SYSTEM.with(|s| s.borrow().cancelled), copy_result)
    }
    fn original_restored(injected: usize) {
        SYSTEM.with(|s| {
            let s = s.borrow();
            assert_eq!(s.injected, injected, "never retry an uncertain chord");
            assert_eq!(s.restored, 1, "uncontended clipboard snapshot must be restored");
            assert_eq!(s.body, "synthetic-original");
        });
    }
    fn new_copy_preserved(injected: usize) {
        SYSTEM.with(|s| {
            let s = s.borrow(); assert_eq!(s.injected, injected);
            assert_eq!(s.restored, 0, "never restore over a real new copy");
            assert_eq!(s.body, "synthetic-new-user-copy");
        });
    }
    #[test]
    fn unchanged_close_allows_one_input_and_restores_snapshot() {
        reset(false); assert_eq!(transaction(false), Ok(true)); original_restored(1);
    }
    #[test]
    fn synthesized_close_is_our_publication_not_a_new_copy() {
        reset(true); assert_eq!(transaction(false), Ok(true)); original_restored(1);
    }
    #[test]
    fn new_owner_on_publication_close_is_never_adopted_as_our_lease() {
        reset(true); SYSTEM.with(|s| s.borrow_mut().copy_on_publish_close = true);
        assert_eq!(transaction(false), Err("manual_paste_clipboard_changed".into()));
        new_copy_preserved(0);
    }
    #[test]
    fn new_owner_without_notification_on_publication_close_still_refuses_input() {
        reset(true);
        SYSTEM.with(|s| {
            let mut s = s.borrow_mut(); s.copy_on_publish_close = true;
            s.suppress_owner_notification = true;
        });
        let mut clipboard = WinClipboard::new(false).unwrap();
        clipboard.prepare("synthetic-transcript").unwrap();
        clipboard.publish().unwrap();
        // The post-close sequence sample may already include a new user's copy.
        // No ownership-loss notification has arrived, so the actual owner check
        // must independently reject it before the injection boundary.
        assert!(!clipboard.context.ownership_lost.get());
        assert_ne!(unsafe { GetClipboardOwner() }.ok(), Some(clipboard.window.hwnd));
        assert_eq!(clipboard.begin_injection(), Err("manual_paste_clipboard_changed"));
        assert!(!clipboard.context.injection_started.get());
        clipboard.settle(false, &|| false).unwrap();
        new_copy_preserved(0);
    }
    #[test]
    fn begin_owner_failure_never_sends_or_retries_input() {
        reset(false); SYSTEM.with(|s| s.borrow_mut().copy_before_begin = true);
        assert_eq!(transaction(false), Err("manual_paste_clipboard_changed".into()));
        new_copy_preserved(0);
    }
    #[test]
    fn real_sequence_change_with_same_owner_still_refuses_input_and_restoration() {
        reset(false); SYSTEM.with(|s| s.borrow_mut().change_before_begin = true);
        assert_eq!(transaction(false), Err("manual_paste_clipboard_changed".into()));
        new_copy_preserved(0);
    }
    #[test]
    fn cancel_after_synthesized_publication_restores_without_input_or_retention() {
        reset(true); SYSTEM.with(|s| s.borrow_mut().cancel_on_publish_close = true);
        assert_eq!(transaction(true), Err(policy::CANCELLED.into())); original_restored(0);
    }
    #[test]
    fn cancel_after_the_single_input_still_restores_without_retry() {
        reset(true); SYSTEM.with(|s| s.borrow_mut().cancel_after_injection = true);
        assert_eq!(transaction(true), Err(policy::CANCELLED.into())); original_restored(1);
    }
    #[test]
    fn new_copy_after_input_wins_even_with_copy_preference() {
        reset(false); SYSTEM.with(|s| s.borrow_mut().copy_after_injection = true);
        assert_eq!(transaction(true), Ok(false)); new_copy_preserved(1);
    }
    #[test]
    fn no_recoverable_original_payload_keeps_current_text_after_one_input() {
        reset(true); SYSTEM.with(|s| s.borrow_mut().restore_unavailable = true);
        assert_eq!(transaction(false), Ok(true));
        SYSTEM.with(|s| { let s = s.borrow(); assert_eq!(s.injected, 1);
            assert_eq!(s.body, "synthetic-transcript"); assert!(!s.delayed);
            assert_eq!(s.restored, 0); });
    }
    #[test]
    fn originally_empty_clipboard_is_restored_to_empty() {
        reset(true); SYSTEM.with(|s| s.borrow_mut().originally_empty = true);
        assert_eq!(transaction(false), Ok(true));
        SYSTEM.with(|s| { let s = s.borrow(); assert_eq!(s.injected, 1);
            assert_eq!(s.body, "empty"); assert!(s.formats.is_empty()); });
    }
    #[test]
    fn failed_format_restoration_keeps_current_text_without_failing_the_input() {
        reset(true); SYSTEM.with(|s| s.borrow_mut().restore_failure = true);
        assert_eq!(transaction(false), Ok(true));
        SYSTEM.with(|s| { let s = s.borrow(); assert_eq!(s.injected, 1);
            assert_eq!(s.body, "synthetic-transcript"); assert!(!s.delayed); });
    }
    #[test]
    fn no_backup_never_overwrites_a_new_user_copy() {
        reset(true); SYSTEM.with(|s| { let mut s = s.borrow_mut();
            s.restore_unavailable = true; s.copy_after_injection = true; });
        assert_eq!(transaction(false), Ok(false)); new_copy_preserved(1);
    }
    #[test]
    fn metadata_markers_alone_do_not_count_as_original_payload() {
        reset(true); SYSTEM.with(|s| s.borrow_mut().marker_only = true);
        assert_eq!(transaction(false), Ok(true));
        SYSTEM.with(|s| { let s = s.borrow(); assert_eq!(s.injected, 1);
            assert_eq!(s.body, "synthetic-transcript"); assert_eq!(s.restored, 0); });
    }
    #[test]
    fn restored_original_html_and_failed_original_text_never_mix_with_current_text() {
        reset(true); SYSTEM.with(|s| { let mut s = s.borrow_mut();
            s.restore_failure = true; s.partial_restore = true; });
        assert_eq!(transaction(false), Ok(true));
        SYSTEM.with(|s| { let s = s.borrow(); assert_eq!(s.injected, 1);
            assert_eq!(s.body, "synthetic-original"); assert_eq!(s.formats, vec![50010]);
            assert!(!s.delayed); assert_eq!(s.restored, 2); });
    }
    #[test]
    fn restored_markers_do_not_mask_failure_of_every_original_payload() {
        reset(true); SYSTEM.with(|s| { let mut s = s.borrow_mut();
            s.restore_failure = true; s.failed_payload_with_markers = true; });
        assert_eq!(transaction(false), Ok(true));
        SYSTEM.with(|s| { let s = s.borrow(); assert_eq!(s.body, "synthetic-transcript");
            assert_eq!(s.formats, vec![13]); assert_eq!(s.injected, 1); });
    }
    #[test]
    fn failed_fallback_text_publication_is_a_real_system_error_without_retry() {
        reset(true); SYSTEM.with(|s| { let mut s = s.borrow_mut();
            s.restore_unavailable = true; s.fail_retention = true; });
        assert_eq!(transaction(false), Err(RESTORE_FAILED.into()));
        SYSTEM.with(|s| assert_eq!(s.borrow().injected, 1));
    }
    #[test]
    fn fallback_text_allocation_failure_precedes_the_first_clipboard_write() {
        reset(true); SYSTEM.with(|s| s.borrow_mut().fail_text_allocation = true);
        assert_eq!(transaction(false), Err(BACKUP_FAILED.into()));
        SYSTEM.with(|s| { let s = s.borrow(); assert_eq!(s.injected, 0);
            assert_eq!(s.body, "synthetic-original"); assert_eq!(s.sequence, 10); });
    }
    #[test]
    fn cancellation_at_settlement_restores_original_instead_of_copy_preference() {
        reset(true); SYSTEM.with(|s| s.borrow_mut().cancel_on_settle = true);
        assert_eq!(transaction(true), Err(policy::CANCELLED.into())); original_restored(1);
    }
    #[test]
    fn cancellation_without_recoverable_payload_still_settles_real_text_and_reports_cancelled() {
        reset(true); SYSTEM.with(|s| { let mut s = s.borrow_mut();
            s.restore_unavailable = true; s.cancel_on_settle = true; });
        assert_eq!(transaction(false), Err(policy::CANCELLED.into()));
        SYSTEM.with(|s| { let s = s.borrow(); assert_eq!(s.body, "synthetic-transcript");
            assert!(!s.delayed); assert_eq!(s.injected, 1); });
    }
}
'''


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", action="store_true")
    parser.add_argument("--rdp", action="store_true", help="also run synthetic redirection schedules")
    parser.add_argument("--rdp-delivery-gate", action="store_true", help="assert delivery for known failing schedules (expected red)")
    args = parser.parse_args()
    if args.rdp_delivery_gate and not args.rdp:
        parser.error("--rdp-delivery-gate requires --rdp")
    filename = "desktop/src-tauri/src/protected_paste_windows.rs"
    source = (subprocess.check_output(["git", "show", f"v0.1.7:{filename}"], cwd=ROOT, text=True, encoding="utf-8")
              if args.baseline else (ROOT / filename).read_text(encoding="utf-8"))
    inherent = source[source.index("impl WinClipboard {"):]
    settlement = source[source.index("impl WinClipboard {\n    fn settle"):]
    bodies = {name: function(source, name) for name in ["current_sequence", "empty", "publish", "begin_injection", "observe", "elapsed", "pause", "render"]}
    bodies["new"] = function(inherent, "new")
    bodies["settle"] = function(settlement, "settle")
    definitions = {name: declaration(source, name) for name in ["Context", "WinClipboard"]}
    policy = (ROOT / "desktop/src-tauri/src/protected_paste_policy.rs").as_posix()
    prepare = function(source, "prepare") if not args.baseline else PREPARE.replace("vec![Saved]", "vec![Saved { format: 13 }]")
    bodies["prepare"] = prepare
    provider = prepare + "\n".join(bodies[name] for name in ["publish", "begin_injection", "observe", "elapsed", "pause"])
    provider += '''
    fn restore(&mut self) -> Result<(), &'static str> { WinClipboard::settle(self, false, &|| false) }
    fn retain_text(&mut self) -> Result<(), &'static str> { WinClipboard::settle(self, true, &|| false) }
    fn ownership_lost(&self) -> bool { self.context.ownership_lost.get() }
    fn settle(&mut self, copy: bool, cancelled: &dyn Fn() -> bool) -> Result<(), &'static str> {
        WinClipboard::settle(self, copy, cancelled)
    }
'''
    harness = ADAPTERS + f'\n#[path = "{policy}"] mod policy;\n'
    harness += "#[derive(Default)]\n" + definitions["Context"] + "\n" + definitions["WinClipboard"]
    harness += "\nimpl WinClipboard {\n" + "\n".join(bodies[name] for name in ["new", "current_sequence", "empty", "settle"]) + "\n}\n"
    harness += "impl Provider for WinClipboard {\n" + provider + "\n}\n" + bodies["render"] + TESTS
    if not args.baseline:
        copy_source = source[source.index("impl CopyLease {"):]
        harness += declaration(source, "CopyLease") + "\nimpl CopyLease {\n"
        harness += function(copy_source, "prepare") + "\n" + function(copy_source, "finish") + "\n}\n"
        harness += (ROOT / "scripts/rdp-copy-cases.rs").read_text(encoding="utf-8")
    rdp_sources = {}
    if args.rdp:
        keyboard = (ROOT / "desktop/src-tauri/src/manual_paste_keyboard.rs").as_posix()
        harness += f'\n#[path = "{keyboard}"] mod keyboard;\n'
        cases = ROOT / "scripts/rdp-paste-cases.rs"
        harness += cases.read_text(encoding="utf-8")
        rdp_sources = {"keyboard_sha256": hashlib.sha256(Path(keyboard).read_bytes()).hexdigest(),
                       "cases_sha256": hashlib.sha256(cases.read_bytes()).hexdigest()}
    out = ROOT / "outputs/win32-publication-018-tests"
    out.mkdir(parents=True, exist_ok=True)
    label = ("baseline" if args.baseline else "current") + ("-rdp" if args.rdp else "")
    if args.rdp_delivery_gate:
        label += "-delivery-gate"
    rustfile = out / f"publication_{label}.rs"
    rustfile.write_text(harness, encoding="utf-8")
    (out / f"manifest_{label}.json").write_text(json.dumps({
        "synthetic_only": True, "native_api_calls": False, "target_insertion_tested": False,
        "source_ref": "v0.1.7" if args.baseline else "working tree",
        "synthetic_rdp_schedules": args.rdp,
        "requires_synthetic_remote_delivery": args.rdp_delivery_gate,
        "rdp_sources": rdp_sources,
        "source_functions": {name: hashlib.sha256(body.encode()).hexdigest() for name, body in bodies.items()},
        "source_structs": {name: hashlib.sha256(body.encode()).hexdigest() for name, body in definitions.items()},
        "policy_sha256": hashlib.sha256(Path(policy).read_bytes()).hexdigest(),
    }, indent=2), encoding="utf-8")
    rustc = shutil.which("rustc") or str(Path.home() / ".cargo/bin/rustc.exe")
    executable = out / (f"publication_{label}.exe" if os.name == "nt" else f"publication_{label}")
    cfg = ["--cfg", "rdp_delivery_gate"] if args.rdp_delivery_gate else []
    compiled = subprocess.run([rustc, "--edition=2021", "--test", *cfg, str(rustfile), "-o", str(executable)], check=False)
    if compiled.returncode:
        return compiled.returncode
    filters = []
    result = subprocess.run([str(executable), *filters], capture_output=True, text=True, timeout=30)
    print(result.stdout, end="")
    print(result.stderr, end="")
    (out / f"{label}-result.txt").write_text(result.stdout + result.stderr, encoding="utf-8")
    return result.returncode


if __name__ == "__main__":
    raise SystemExit(main())
