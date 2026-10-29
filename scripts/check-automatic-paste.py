"""Exercise production automatic paste with synthetic clipboard/keyboard adapters.

No clipboard, keyboard, audio, history, network, Tauri runtime, or ASR API is used.
Production function bodies are extracted verbatim except that their sleeps advance
one deterministic virtual clock. The adapters model a target reading clipboard
data after receiving an enqueued paste chord; they do not prove a real target's
read latency. --legacy runs the original red regression against v0.1.5;
--characterize documents that baseline. Default tests the current safe route.
"""
from pathlib import Path
import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess


ROOT = Path(__file__).resolve().parents[1]


def function(source: str, name: str) -> str:
    match = re.search(rf"\bfn {re.escape(name)}(?:<[^\n]*>)?\(", source)
    if match is None:
        raise ValueError(f"Production function not found: {name}")
    start = match.start()
    line_start = source.rfind("\n", 0, start) + 1
    # Match braces lexically here: these selected production bodies contain no
    # unmatched braces inside strings/comments. Assert the end at a line boundary.
    opening = source.index("{", start)
    depth = 1
    end = opening + 1
    while depth:
        depth += (source[end] == "{") - (source[end] == "}")
        end += 1
    assert source[end:end + 1] in ("\n", "")
    return source[line_start:end]


ADAPTERS = r'''
#![allow(dead_code, unused_imports, unused_variables, unused_macros)]
use std::cell::RefCell;
use std::sync::{Arc, Mutex};
use std::time::Duration;
macro_rules! info { ($($args:tt)*) => { () } }

#[derive(Clone, Copy, Debug)]
enum PasteMethod { None, CtrlV, CtrlShiftV, ShiftInsert, Direct, ExternalScript }
#[derive(Clone, Copy, PartialEq)]
enum ClipboardHandling { DontModify, CopyToClipboard }
#[derive(Clone, Copy)]
struct Settings {
    paste_method: PasteMethod, paste_delay_ms: u64, paste_delay_after_ms: u64,
    append_trailing_space: bool, clipboard_handling: ClipboardHandling,
    reliable_paste: bool, auto_submit_key: (),
}
impl Default for Settings {
    fn default() -> Self { Self { paste_method: PasteMethod::CtrlV,
        paste_delay_ms: default_paste_delay_ms(), paste_delay_after_ms: default_paste_delay_after_ms(), append_trailing_space: false,
        clipboard_handling: ClipboardHandling::DontModify, reliable_paste: false,
        auto_submit_key: (),
    } }
}
fn get_settings(app: &AppHandle) -> Settings { app.settings }

#[derive(Default)]
struct SyntheticDesktop {
    now: u64, clipboard: String, read_after_chord: u64,
    pending_reads: Vec<u64>, received: Vec<String>,
    writes: Vec<(u64, String)>, chord_times: Vec<u64>,
    reliable_calls: usize,
    protected_calls: usize, legacy_writes: usize, read_at: Option<u64>,
    cancelled: bool, cancel_at: Option<u64>, backup_wait: u64,
    backup_failure: bool, restore_failure: bool, keys_held: bool,
    copy_at: Option<u64>, copy_on_settle: bool, ownership_lost: bool,
    retained: bool, published_text: String,
    settle_wait: u64, events: Vec<(&'static str, &'static str)>,
    preflight_wait: u64,
    copy_on_input_lock: bool,
    rdp: bool,
    unicode_calls: usize,
}
thread_local! { static DESKTOP: RefCell<SyntheticDesktop> = RefCell::new(SyntheticDesktop::default()); }
fn advance(delta: u64) {
    DESKTOP.with(|desktop| {
        let mut state = desktop.borrow_mut();
        let end = state.now + delta;
        if state.cancel_at.is_some_and(|at| at <= end) { state.cancelled = true; }
        if state.copy_at.is_some_and(|at| at <= end) {
            state.clipboard = "synthetic-new-user-copy".into();
            state.ownership_lost = true; state.copy_at = None;
        }
        let due: Vec<u64> = state.pending_reads.iter().copied().filter(|&at| at <= end).collect();
        for at in due {
            let text = state.clipboard.clone();
            // Model only the observed failing transport boundary, not a claim
            // about MSTSC's internal code page or conversion implementation.
            let received = if state.rdp {
                text.chars().map(|c| if c.is_ascii() { c } else { '?' }).collect()
            } else { text };
            state.received.push(received);
            if !state.ownership_lost && state.clipboard == state.published_text && state.read_at.is_none() {
                state.read_at = Some(at);
            }
        }
        state.pending_reads.retain(|&at| at > end);
        state.now = end;
    });
}
mod virtual_clock {
    pub fn sleep(duration: std::time::Duration) { super::advance(duration.as_millis() as u64); }
}
#[derive(Clone)]
struct FakeImage;
struct FakeClipboard;
impl FakeClipboard {
    fn read_text(&self) -> Result<String, String> { Ok(DESKTOP.with(|s| s.borrow().clipboard.clone())) }
    fn read_image(&self) -> Result<FakeImage, String> { Err("synthetic text only".into()) }
    fn write_text(&self, text: &str) -> Result<(), String> {
        DESKTOP.with(|s| { let mut s = s.borrow_mut(); s.legacy_writes += 1; s.clipboard = text.into();
            let at = s.now; s.writes.push((at, text.into())); }); Ok(())
    }
    fn write_image(&self, _: &FakeImage) -> Result<(), String> { unreachable!() }
    fn clear(&self) -> Result<(), String> { self.write_text("") }
}
#[derive(Clone)]
struct AppHandle { settings: Settings, enigo: Arc<input::EnigoState>, lock_barrier: Option<Arc<std::sync::Barrier>> }
struct PasteErrorEvent { code: &'static str }
impl AppHandle {
    fn emit(&self, event: &'static str, payload: PasteErrorEvent) -> Result<(), ()> {
        DESKTOP.with(|s| s.borrow_mut().events.push((event, payload.code))); Ok(())
    }
    fn clipboard(&self) -> FakeClipboard { FakeClipboard }
    fn try_state<T: 'static>(&self) -> Option<&T> {
        DESKTOP.with(|s| { let mut s=s.borrow_mut(); if s.copy_on_input_lock {
            s.copy_on_input_lock=false; s.clipboard="synthetic-new-user-copy".into(); s.ownership_lost=true;
        }});
        if let Some(barrier) = &self.lock_barrier { barrier.wait(); }
        let state: &dyn std::any::Any = &*self.enigo; state.downcast_ref()
    }
}
mod enigo {
    #[derive(Clone, Copy, Debug)] pub enum Key { Control, Shift, Meta, Other(u32), Unicode(char) }
    #[derive(Clone, Copy, Debug)] pub enum Direction { Press, Release, Click }
    pub struct Enigo;
    pub trait Keyboard { fn key(&mut self, key: Key, direction: Direction) -> Result<(), String>; }
    impl Keyboard for Enigo {
        fn key(&mut self, key: Key, direction: Direction) -> Result<(), String> {
            if matches!(direction, Direction::Click) ||
                (matches!(direction, Direction::Press) && matches!(key, Key::Other(0x56 | 0x2d))) {
                super::DESKTOP.with(|s| { let mut s = s.borrow_mut(); let at = s.now;
                    let read_at = at + s.read_after_chord;
                    s.chord_times.push(at); s.pending_reads.push(read_at); });
            }
            super::advance(0);
            Ok(())
        }
    }
}
use enigo::{Enigo, Key, Keyboard};
use input::EnigoState;
mod paste_tx {
    pub fn try_reliable_paste(_: &str, _: &super::AppHandle, _: &super::PasteMethod,
        _: &mut super::Enigo, _: bool, _: (), _: super::ClipboardHandling) -> Result<(), String> {
        super::DESKTOP.with(|s| s.borrow_mut().reliable_calls += 1);
        Err("automatic paste must not reach debug reliable path".into())
    }
}
'''

SAFE_ADAPTERS = r'''
mod settings {
    pub(crate) use super::PasteMethod;
    pub(crate) fn get_settings(app: &super::AppHandle) -> super::Settings { super::get_settings(app) }
}
mod manual_paste {
    use super::*;
    use crate::keyboard;
    fn any_keyboard_key_down() -> Result<bool, &'static str> {
        Ok(DESKTOP.with(|s| s.borrow().keys_held))
    }
    pub(crate) struct PreparedKeyboard;
    fn bound_high_keys(_: &Settings) -> u16 { 0 }
    pub(crate) fn wait_before_automatic(_: &AppHandle, cancelled: impl Fn() -> bool) -> Result<(), String> {
        if cancelled() { return Err("manual_paste_cancelled".into()); }
        if any_keyboard_key_down().unwrap() { advance(400); return Err("keys_held".into()); }
        Ok(())
    }
    pub(crate) fn prepare_keyboard(_: &AppHandle) -> Result<PreparedKeyboard, String> {
        advance(DESKTOP.with(|s| s.borrow().preflight_wait));
        if any_keyboard_key_down().unwrap() { Err("keys_held".into()) } else { Ok(PreparedKeyboard) }
    }
    pub(crate) fn keyboard_is_held(_: &AppHandle, _: &PreparedKeyboard) -> Result<bool, &'static str> {
        any_keyboard_key_down()
    }
    mod keyboard_state_windows {
        pub(super) fn fast_check(_: &super::PreparedKeyboard, _: u16) -> Result<bool, &'static str> {
            super::any_keyboard_key_down()
        }
    }
    // REAL_MANUAL_CHORD
}
mod protected_paste {
    use super::*;
    mod platform {
        use super::*;
        struct FakeProvider { original: String, text: String, began: u64 }
        impl crate::protected_policy::Provider for FakeProvider {
            fn prepare(&mut self, text: &str) -> Result<(), &'static str> {
                let delay = DESKTOP.with(|s| s.borrow().backup_wait);
                advance(delay);
                if DESKTOP.with(|s| s.borrow().backup_failure) { return Err("manual_paste_backup_failed"); }
                self.original = DESKTOP.with(|s| s.borrow().clipboard.clone());
                self.text = text.into(); Ok(())
            }
            fn publish(&mut self) -> Result<(), &'static str> {
                self.began = DESKTOP.with(|s| s.borrow().now);
                DESKTOP.with(|s| { let mut s = s.borrow_mut();
                    s.clipboard = self.text.clone(); s.published_text = self.text.clone();
                    s.read_at = None; s.ownership_lost = false; s.retained = false;
                    let at = s.now; s.writes.push((at, self.text.clone())); });
                Ok(())
            }
            fn begin_injection(&mut self) -> Result<(), &'static str> {
                if self.ownership_lost() { Err("manual_paste_clipboard_changed") } else { Ok(()) }
            }
            fn observe(&mut self) -> Result<crate::protected_policy::Observation, &'static str> {
                DESKTOP.with(|s| { let s = s.borrow(); Ok(crate::protected_policy::Observation {
                    read: s.read_at.is_some(),
                    quiet_for: s.read_at.map(|at| Duration::from_millis(s.now - at)),
                    ownership_lost: s.ownership_lost,
                }) })
            }
            fn elapsed(&self) -> Duration { Duration::from_millis(DESKTOP.with(|s| s.borrow().now) - self.began) }
            fn pause(&mut self) { advance(10); }
            fn restore(&mut self) -> Result<(), &'static str> {
                self.settle(false)
            }
            fn retain_text(&mut self) -> Result<(), &'static str> {
                self.settle(true)
            }
            fn ownership_lost(&self) -> bool { DESKTOP.with(|s| s.borrow().ownership_lost) }
            fn settle(&mut self, copy: bool, cancelled: &dyn Fn() -> bool) -> Result<(), &'static str> {
                let wait = DESKTOP.with(|s| s.borrow().settle_wait);
                advance(wait); // Acquiring the final settlement lock can wait.
                let is_cancelled = crate::protected_policy::settlement_cancelled(cancelled);
                let action = crate::protected_policy::settlement_action(copy, !self.ownership_lost(), is_cancelled);
                self.settle(action == crate::protected_policy::SettlementAction::RetainResult)?;
                crate::protected_policy::settlement_result(is_cancelled)
            }
        }
        impl FakeProvider {
            fn settle(&mut self, keep: bool) -> Result<(), &'static str> {
                DESKTOP.with(|s| { let mut s = s.borrow_mut();
                    if s.restore_failure { return Err("manual_paste_restore_failed"); }
                    if s.copy_on_settle { s.clipboard = "synthetic-new-user-copy".into(); s.ownership_lost = true; }
                    if !s.ownership_lost {
                        s.clipboard = if keep { self.text.clone() } else { self.original.clone() };
                        s.retained = keep;
                        let at = s.now; let text = s.clipboard.clone(); s.writes.push((at, text));
                    }
                    Ok(())
                })
            }
        }
        pub(super) fn run_with_preflight(text: &str, preflight: impl FnOnce() -> Result<(), String>, inject: impl FnOnce() -> Result<(), String>,
            cancelled: impl Fn() -> bool, copy: bool) -> Result<bool, String> {
            DESKTOP.with(|s| s.borrow_mut().protected_calls += 1);
            let mut provider = FakeProvider { original: String::new(), text: String::new(), began: 0 };
            crate::protected_policy::transact_with_preflight(&mut provider, text, preflight, inject, cancelled, copy)
        }
    }
    // REAL_AUTOMATIC_WRAPPER
}
'''

SAFE_TESTS = r'''
#[cfg(test)]
mod automatic_tests {
    use super::*;
    // Older local-path assertions use a boolean. Production returns evidence;
    // retain those assertions while testing the real coordinator unchanged.
    fn paste_cancellable(text: String, app: AppHandle, cancelled: impl Fn() -> bool) -> Result<bool, String> {
        super::paste_cancellable(text, app, cancelled).map(|e| e != Submission::Cancelled)
    }
    fn app(delay: u64, method: PasteMethod, copy: bool) -> AppHandle {
        DESKTOP.with(|s| *s.borrow_mut() = SyntheticDesktop {
            clipboard: "synthetic-previous-transcript".into(), read_after_chord: delay, ..Default::default()
        });
        AppHandle { settings: Settings { paste_method: method,
            clipboard_handling: if copy { ClipboardHandling::CopyToClipboard } else { ClipboardHandling::DontModify },
            ..Default::default() }, enigo: Arc::new(EnigoState(Mutex::new(Enigo))), lock_barrier: None }
    }
    fn cancelled() -> bool { DESKTOP.with(|s| s.borrow().cancelled) }
    #[test]
    fn mstsc_unicode_body_bypasses_the_observed_lossy_clipboard_boundary() {
        let app = app(100, PasteMethod::CtrlV, false);
        DESKTOP.with(|s| s.borrow_mut().rdp = true);
        assert_eq!(super::paste_cancellable("轻述远程输入测试 RDP T1".into(), app, cancelled), Ok(Submission::UnicodeSubmitted));
        DESKTOP.with(|s| { let s = s.borrow();
            assert_eq!(s.received, ["轻述远程输入测试 RDP T1"]);
            assert_eq!(s.protected_calls, 0);
            assert!(s.writes.is_empty());
            assert!(s.chord_times.is_empty());
            assert_eq!(s.unicode_calls, 1);
        });
    }
    #[test]
    fn mstsc_manual_recovery_normalizes_text_and_ignores_automatic_copy_preference() {
        let app = app(100, PasteMethod::CtrlV, true);
        DESKTOP.with(|s| s.borrow_mut().rdp = true);
        assert_eq!(paste_latest_preserving_clipboard("中\r\nA\t😀 C:\\tmp\\n".into(), app, cancelled), Ok(false));
        DESKTOP.with(|s| { let s=s.borrow();
            assert_eq!(s.received, ["中 A 😀 C:\\tmp\\n"]);
            assert_eq!(s.clipboard, "synthetic-previous-transcript");
            assert_eq!((s.unicode_calls,s.protected_calls,s.legacy_writes),(1,0,0));
            assert!(s.chord_times.is_empty()); assert!(s.writes.is_empty());
        });
    }
    #[test]
    fn mstsc_cancel_during_preflight_has_no_input_or_clipboard_effect() {
        let app = app(100, PasteMethod::CtrlV, false);
        DESKTOP.with(|s| { let mut s=s.borrow_mut(); s.rdp=true; s.preflight_wait=60; s.cancel_at=Some(30); });
        assert_eq!(super::paste_cancellable("中".into(),app,cancelled),Ok(Submission::Cancelled));
        DESKTOP.with(|s| { let s=s.borrow(); assert_eq!((s.unicode_calls,s.protected_calls,s.legacy_writes),(0,0,0));
            assert!(s.received.is_empty()); assert!(s.writes.is_empty()); });
    }
    #[test]
    fn mstsc_large_text_refuses_before_any_submission_without_clipboard_fallback() {
        let app = app(100, PasteMethod::CtrlV, false);
        DESKTOP.with(|s| s.borrow_mut().rdp=true);
        assert_eq!(super::paste_cancellable("中".repeat(4097),app,cancelled),Err("rdp_input_too_long".into()));
        DESKTOP.with(|s| { let s=s.borrow(); assert_eq!((s.unicode_calls,s.protected_calls,s.legacy_writes),(0,0,0));
            assert!(s.chord_times.is_empty()); });
    }
    #[test]
    fn local_receipt_and_mstsc_disabled_mode_keep_distinct_evidence() {
        let local=app(100,PasteMethod::CtrlV,false);
        assert_eq!(super::paste_cancellable("中".into(),local,cancelled),Ok(Submission::ClipboardReadObserved));
        let remote=app(100,PasteMethod::None,false);
        DESKTOP.with(|s| s.borrow_mut().rdp=true);
        assert_eq!(super::paste_cancellable("中".into(),remote,cancelled),Err("automatic_paste_disabled".into()));
        DESKTOP.with(|s| assert_eq!(s.borrow().unicode_calls,0));
    }
    #[test]
    fn regression_normal_completion_delivers_current_result_to_slow_reader() {
        for method in [PasteMethod::CtrlV, PasteMethod::CtrlShiftV, PasteMethod::ShiftInsert] {
            for delay in [161, 500, 1000, 2500] {
                let app = app(delay, method, false);
                assert_eq!(paste_cancellable("synthetic-current-transcript".into(), app, cancelled), Ok(true));
                DESKTOP.with(|s| { let s = s.borrow();
                    assert_eq!(s.received, vec!["synthetic-current-transcript"]);
                    assert_eq!(s.clipboard, "synthetic-previous-transcript");
                    assert_eq!(s.chord_times.len(), 1);
                    assert_eq!(s.protected_calls, 1); assert_eq!(s.legacy_writes, 0);
                });
            }
        }
    }
    #[test]
    fn distinct_consecutive_completed_results_are_never_replaced_by_latest_history() {
        let app = app(500, PasteMethod::CtrlV, false);
        for text in ["synthetic-current-1", "synthetic-current-2"] {
            assert_eq!(paste_cancellable(text.into(), app.clone(), cancelled), Ok(true));
        }
        DESKTOP.with(|s| { let s = s.borrow();
            assert_eq!(s.received, vec!["synthetic-current-1", "synthetic-current-2"]);
            assert_eq!(s.protected_calls, 2); assert_eq!(s.chord_times.len(), 2);
        });
    }
    #[test]
    fn cancellation_during_backup_never_publishes_or_sends_keys() {
        let app = app(500, PasteMethod::CtrlV, false);
        DESKTOP.with(|s| { let mut s = s.borrow_mut(); s.backup_wait = 50; s.cancel_at = Some(20); });
        assert_eq!(paste_cancellable("synthetic-current".into(), app, cancelled), Ok(false));
        DESKTOP.with(|s| { let s = s.borrow(); assert!(s.chord_times.is_empty());
            assert!(s.writes.is_empty()); assert_eq!(s.clipboard, "synthetic-previous-transcript"); });
    }
    #[test]
    fn cancellation_while_waiting_for_actual_enigo_lock_never_touches_clipboard() {
        use std::sync::{Barrier, atomic::{AtomicBool, Ordering}};
        let mut app = app(500, PasteMethod::CtrlV, false);
        let state = Arc::clone(&app.enigo);
        let cancelled = Arc::new(AtomicBool::new(false)); let cancelled_worker = Arc::clone(&cancelled);
        let barrier = Arc::new(Barrier::new(2)); let barrier_worker = Arc::clone(&barrier);
        app.lock_barrier = Some(Arc::clone(&barrier));
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let holder = std::thread::spawn(move || {
            let lock = state.0.lock().unwrap(); ready_tx.send(()).unwrap();
            barrier_worker.wait(); cancelled_worker.store(true, Ordering::SeqCst); drop(lock);
        });
        ready_rx.recv().unwrap();
        let result = paste_cancellable("synthetic-current".into(), app, || cancelled.load(Ordering::SeqCst));
        holder.join().unwrap(); assert_eq!(result, Ok(false));
        DESKTOP.with(|s| { let s = s.borrow(); assert_eq!(s.protected_calls, 0);
            assert_eq!(s.legacy_writes, 0); assert!(s.chord_times.is_empty()); });
    }
    #[test]
    fn backup_and_keyboard_rejections_never_fall_back_to_timer_paste() {
        for backup in [true, false] {
            let app = app(500, PasteMethod::CtrlV, false);
            DESKTOP.with(|s| { let mut s = s.borrow_mut(); s.backup_failure = backup; s.keys_held = !backup; });
            let expected = if backup { "manual_paste_backup_failed" } else { "keys_held" };
            assert_eq!(paste_cancellable("synthetic-current".into(), app, cancelled), Err(expected.into()));
            DESKTOP.with(|s| { let s = s.borrow(); assert!(s.chord_times.is_empty());
                assert_eq!(s.legacy_writes, 0); assert_eq!(s.clipboard, "synthetic-previous-transcript"); });
        }
    }
    #[test]
    fn new_copy_during_keyboard_preflight_is_rechecked_before_any_chord() {
        let app = app(500, PasteMethod::CtrlV, false);
        DESKTOP.with(|s| { let mut s = s.borrow_mut(); s.preflight_wait = 60; s.copy_at = Some(30); });
        assert_eq!(paste_cancellable("synthetic-current".into(), app, cancelled),
            Err("manual_paste_clipboard_changed".into()));
        DESKTOP.with(|s| { let s = s.borrow(); assert!(s.chord_times.is_empty());
            assert_eq!(s.clipboard, "synthetic-new-user-copy"); assert_eq!(s.legacy_writes, 0); });
    }
    #[test]
    fn cancellation_during_keyboard_preflight_is_settled_without_a_chord() {
        let app = app(500, PasteMethod::CtrlV, false);
        DESKTOP.with(|s| { let mut s = s.borrow_mut(); s.preflight_wait = 60; s.cancel_at = Some(30); });
        assert_eq!(paste_cancellable("synthetic-current".into(), app, cancelled), Ok(false));
        DESKTOP.with(|s| { let s = s.borrow(); assert!(s.chord_times.is_empty());
            assert_eq!(s.clipboard, "synthetic-previous-transcript"); });
    }
    #[test]
    fn manual_input_lock_is_acquired_before_clipboard_backup_publication_and_lease() {
        let app=app(500,PasteMethod::CtrlV,false);
        DESKTOP.with(|s|s.borrow_mut().copy_on_input_lock=true);
        assert_eq!(paste_latest_preserving_clipboard("synthetic-current".into(),app,cancelled),Ok(true));
        DESKTOP.with(|s| { let s=s.borrow(); assert_eq!(s.received,vec!["synthetic-current"]);
            assert_eq!(s.clipboard,"synthetic-new-user-copy"); assert_eq!(s.chord_times.len(),1); });
    }
    #[test]
    fn manual_new_copy_during_preflight_is_rejected_before_any_chord() {
        let app=app(500,PasteMethod::CtrlV,false);
        DESKTOP.with(|s| {let mut s=s.borrow_mut();s.preflight_wait=60;s.copy_at=Some(30);});
        assert_eq!(paste_latest_preserving_clipboard("synthetic-current".into(),app,cancelled),Err("manual_paste_clipboard_changed".into()));
        DESKTOP.with(|s| {let s=s.borrow();assert!(s.chord_times.is_empty());assert_eq!(s.clipboard,"synthetic-new-user-copy");});
    }
    #[test]
    fn cancellation_after_chord_restores_without_retry() {
        let app = app(500, PasteMethod::CtrlV, false);
        DESKTOP.with(|s| s.borrow_mut().cancel_at = Some(50));
        assert_eq!(paste_cancellable("synthetic-current".into(), app, cancelled), Ok(false));
        DESKTOP.with(|s| { let s = s.borrow(); assert_eq!(s.chord_times.len(), 1);
            assert_eq!(s.clipboard, "synthetic-previous-transcript"); assert_eq!(s.protected_calls, 1); });
    }
    #[test]
    fn timeout_is_unconfirmed_and_does_not_retry_even_when_copy_is_enabled() {
        for copy in [false, true] {
            let app = app(10_000, PasteMethod::CtrlV, copy);
            assert_eq!(paste_cancellable("synthetic-current".into(), app, cancelled),
                Err("automatic_paste_unconfirmed".into()));
            DESKTOP.with(|s| { let s = s.borrow(); assert_eq!(s.chord_times.len(), 1);
                assert_eq!(s.protected_calls, 1); assert_eq!(s.legacy_writes, 0);
                assert_eq!(s.clipboard, if copy { "synthetic-current" } else { "synthetic-previous-transcript" }); });
        }
    }
    #[test]
    fn concurrent_user_copy_wins_even_after_a_read_or_during_settlement() {
        for copy in [false, true] {
            for during_settlement in [false, true] {
                let app = app(10, PasteMethod::CtrlV, copy);
                DESKTOP.with(|s| { let mut s = s.borrow_mut();
                    if during_settlement { s.copy_on_settle = true; } else { s.copy_at = Some(120); } });
                assert_eq!(paste_cancellable("synthetic-current".into(), app, cancelled),
                    Err("automatic_paste_unconfirmed".into()));
                DESKTOP.with(|s| { let s = s.borrow(); assert_eq!(s.received, vec!["synthetic-current"]);
                    assert_eq!(s.clipboard, "synthetic-new-user-copy"); assert_eq!(s.chord_times.len(), 1);
                    assert_eq!(s.legacy_writes, 0); });
            }
        }
    }
    #[test]
    fn copy_preference_settles_current_text_inside_the_lease() {
        let app = app(500, PasteMethod::CtrlV, true);
        assert_eq!(paste_cancellable("synthetic-current".into(), app, cancelled), Ok(true));
        DESKTOP.with(|s| { let s = s.borrow(); assert!(s.retained);
            assert_eq!(s.clipboard, "synthetic-current"); assert_eq!(s.legacy_writes, 0); });
    }
    #[test]
    fn restore_failure_overrides_read_receipt() {
        let app = app(10, PasteMethod::CtrlV, false);
        DESKTOP.with(|s| s.borrow_mut().restore_failure = true);
        assert_eq!(paste_cancellable("synthetic-current".into(), app, cancelled),
            Err("manual_paste_restore_failed".into()));
        DESKTOP.with(|s| assert_eq!(s.borrow().chord_times.len(), 1));
    }
    #[test]
    fn disabled_paste_does_not_claim_a_clipboard_read() {
        let app = app(10, PasteMethod::None, false);
        assert_eq!(paste_cancellable("synthetic-current".into(), app, cancelled), Err("automatic_paste_disabled".into()));
        DESKTOP.with(|s| { let s = s.borrow(); assert!(s.chord_times.is_empty()); assert_eq!(s.protected_calls, 0); });
    }
    #[test]
    fn cancel_while_waiting_for_settlement_lock_restores_instead_of_retaining() {
        for copy in [false, true] {
            let app = app(10, PasteMethod::CtrlV, copy);
            DESKTOP.with(|s| { let mut s = s.borrow_mut(); s.settle_wait = 100; s.cancel_at = Some(200); });
            assert_eq!(paste_cancellable("synthetic-current".into(), app, cancelled), Ok(false));
            DESKTOP.with(|s| { let s = s.borrow(); assert!(!s.retained);
                assert_eq!(s.clipboard, "synthetic-previous-transcript");
                assert_eq!(s.chord_times.len(), 1); });
        }
    }
    #[test]
    fn real_actions_error_helper_emits_only_whitelisted_codes() {
        let app = app(10, PasteMethod::CtrlV, false);
        for error in ["manual_paste_restore_failed", "automatic_paste_unconfirmed", "private synthetic detail"] {
            emit_paste_error(&app, error);
        }
        DESKTOP.with(|s| assert_eq!(s.borrow().events, vec![
            ("paste-error", "clipboard_restore_failed"),
            ("paste-error", "input_unconfirmed"), ("paste-error", "failed")]));
    }
}
'''

TESTS = r'''
#[cfg(test)]
mod synthetic_tests {
    use super::*;
    fn app(delay: u64, method: PasteMethod, copy: bool, reliable: bool) -> AppHandle {
        DESKTOP.with(|s| *s.borrow_mut() = SyntheticDesktop {
            clipboard: "synthetic-previous-transcript".into(),
            read_after_chord: delay, ..Default::default()
        });
        AppHandle { settings: Settings { paste_method: method,
            clipboard_handling: if copy { ClipboardHandling::CopyToClipboard } else { ClipboardHandling::DontModify },
            reliable_paste: reliable, ..Default::default() },
            enigo: Arc::new(EnigoState(Mutex::new(Enigo))), lock_barrier: None }
    }
    fn deliver(app: AppHandle, text: &str) -> (Result<bool, String>, Vec<String>) {
        let result = paste_cancellable(text.into(), app, || false);
        advance(10_000);
        (result, DESKTOP.with(|s| s.borrow().received.clone()))
    }
    #[test]
    fn regression_normal_completion_must_deliver_this_result_to_late_reader() {
        let app = app(161, PasteMethod::CtrlV, false, false);
        let (result, received) = deliver(app, "synthetic-current-transcript");
        assert_eq!(result, Ok(true));
        assert_eq!(received, vec!["synthetic-current-transcript"],
            "normal completion reported success but target received an older transcript");
    }
    #[test]
    fn characterize_actual_timer_boundary_and_three_chords() {
        for method in [PasteMethod::CtrlV, PasteMethod::CtrlShiftV, PasteMethod::ShiftInsert] {
            for delay in [0, 100, 159, 160, 161, 500, 1000] {
                let app = app(delay, method, false, false);
                let (result, received) = deliver(app, "synthetic-current-transcript");
                assert_eq!(result, Ok(true));
                let expected = if delay <= 160 { "synthetic-current-transcript" }
                    else { "synthetic-previous-transcript" };
                assert_eq!(received, vec![expected]);
                DESKTOP.with(|s| { let s = s.borrow();
                    assert_eq!(s.chord_times, vec![60]);
                    assert_eq!(s.writes, vec![(0, "synthetic-current-transcript".into()),
                        (220, "synthetic-previous-transcript".into())]);
                    assert_eq!(s.reliable_calls, 0);
                });
            }
        }
    }
    #[test]
    fn characterize_repeated_normal_results_can_all_paste_previous_clipboard() {
        let app = app(500, PasteMethod::CtrlV, false, false);
        for current in ["synthetic-current-1", "synthetic-current-2", "synthetic-current-3"] {
            assert_eq!(paste_cancellable(current.into(), app.clone(), || false), Ok(true));
            advance(1000);
        }
        DESKTOP.with(|s| assert_eq!(s.borrow().received,
            vec!["synthetic-previous-transcript"; 3]));
    }
    #[test]
    fn characterize_copy_preference_hides_late_read_race() {
        let app = app(500, PasteMethod::CtrlV, true, false);
        assert_eq!(deliver(app, "synthetic-current-transcript").1,
            vec!["synthetic-current-transcript"]);
    }
    #[test]
    fn characterize_debug_reliable_setting_does_not_change_automatic_route() {
        let app = app(500, PasteMethod::CtrlV, false, true);
        assert_eq!(deliver(app, "synthetic-current-transcript").1,
            vec!["synthetic-previous-transcript"]);
        DESKTOP.with(|s| assert_eq!(s.borrow().reliable_calls, 0));
    }
}
'''


def rdp_adapters(source):
    policy = (source / "rdp_input.rs").read_text(encoding="utf-8")
    policy = policy.replace('#[cfg(target_os = "windows")]\n#[path = "rdp_input_windows.rs"]\npub(crate) mod platform;\n', '')
    return '\nmod rdp_input {\n' + policy + r'''
pub(crate) mod platform {
    use super::*;
    use crate::{DESKTOP, advance};
    pub(crate) struct Target;
    pub(crate) fn capture() -> Result<Option<Target>, String> {
        Ok(DESKTOP.with(|s| s.borrow().rdp).then_some(Target))
    }
    struct Adapter<P, K, C> { prepare: Option<K>, check: C, token: Option<P>,
        copy: bool, original: String, events: Vec<UnicodeEvent> }
    impl<P, K, C> Runtime for Adapter<P, K, C>
    where K: FnOnce() -> Result<P, &'static str>, C: Fn(&P) -> Result<(), &'static str> {
        fn prepare(&mut self, _: &str, events: &[UnicodeEvent]) -> Result<(), &'static str> {
            self.events = events.into();
            if self.copy { self.original = DESKTOP.with(|s| s.borrow().clipboard.clone()); }
            self.token = Some(self.prepare.take().unwrap()()?); Ok(())
        }
        fn ready(&self) -> Result<(), &'static str> { (self.check)(self.token.as_ref().unwrap()) }
        fn submit(&mut self) -> usize {
            let units: Vec<_> = self.events.iter().filter(|e| !e.key_up).map(|e| e.unit).collect();
            DESKTOP.with(|s| { let mut s = s.borrow_mut(); s.unicode_calls += 1;
                s.received.push(String::from_utf16(&units).unwrap()); }); self.events.len()
        }
        fn finish_copy(&mut self, cancelled: &dyn Fn() -> bool) -> Result<(), &'static str> {
            if self.copy && !cancelled() {
                DESKTOP.with(|s| { let mut s = s.borrow_mut(); if s.clipboard == self.original {
                    let text = s.received.last().unwrap().clone(); s.clipboard = text; }
                });
            }
            Ok(())
        }
    }
    pub(crate) fn submit<P>(_: Target, text: &str, copy: bool, cancelled: impl Fn() -> bool,
        prepare: impl FnOnce() -> Result<P, &'static str>, check: impl Fn(&P) -> Result<(), &'static str>) -> Result<(), String> {
        let mut runtime = Adapter { prepare: Some(prepare), check, token: None, copy, original: String::new(), events: Vec::new() };
        super::run(&mut runtime, text, cancelled).map_err(String::from)
    }
}
}
'''


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--characterize", action="store_true")
    parser.add_argument("--legacy", action="store_true")
    parser.add_argument("--late-preflight", action="store_true", help="Model the old unsafe placement after the lease gate; regression must fail")
    parser.add_argument("--filter", default="")
    args = parser.parse_args()
    source = ROOT / "desktop/src-tauri/src"
    selected = {
        "clipboard.rs": ["with_enigo", "write_text_to_clipboard", "finish_clipboard_paste",
                         "paste_via_clipboard", "validate_paste_method", "paste_cancellable",
                         "paste_with_cancellation"],
        "input.rs": ["send_paste_ctrl_v", "send_paste_ctrl_shift_v", "send_paste_shift_insert"],
        "dictation_lifecycle.rs": ["normalize_output"],
        "settings.rs": ["default_paste_delay_ms", "default_paste_delay_after_ms"],
    }
    baseline = args.legacy or args.characterize
    if not baseline:
        selected["clipboard.rs"].extend(["paste_automatic_windows", "paste_latest_preserving_clipboard", "submit_mstsc"])
        selected["actions.rs"] = ["emit_paste_error"]
    functions = {}
    manifest = {"synthetic_only": True, "source_functions": {},
                "adaptation": "std::thread::sleep -> virtual_clock::sleep only",
                "source_ref": "v0.1.5" if baseline else "working tree"}
    for filename, names in selected.items():
        if baseline:
            text = subprocess.check_output(
                ["git", "show", f"v0.1.5:desktop/src-tauri/src/{filename}"],
                cwd=ROOT, text=True, encoding="utf-8")
        else:
            text = (source / filename).read_text(encoding="utf-8")
        functions[filename] = []
        for name in names:
            body = function(text, name)
            manifest["source_functions"][f"{filename}::{name}"] = hashlib.sha256(body.encode()).hexdigest()
            functions[filename].append(body.replace("std::thread::sleep", "crate::virtual_clock::sleep"))
    delivery = (source / "paste_delivery.rs").as_posix()
    safe_adapters = ""
    if not baseline:
        actions = (source / "actions.rs").read_text(encoding="utf-8")
        assert actions.count("emit_paste_error(&ah_clone, &e)") == 3
        assert actions.count("emit_paste_error(&ah, &e)") == 1
        assert 'emit("paste-error", ())' not in actions
        windows_adapter = (source / "protected_paste_windows.rs").read_text(encoding="utf-8")
        locked_settlement = windows_adapter[windows_adapter.index("impl WinClipboard {\n    fn settle("):]
        lock_open = locked_settlement.index("ClipboardLock::open(self.window.hwnd)")
        cancel_sample = locked_settlement.index("policy::settlement_cancelled(is_cancelled)", lock_open)
        first_empty = locked_settlement.index("self.empty()")
        assert lock_open < cancel_sample < first_empty, "final cancellation gate must follow lock acquisition and precede mutation"
        keyboard = (source / "manual_paste_keyboard.rs").as_posix()
        protected = (source / "protected_paste_policy.rs").as_posix()
        chord = function((source / "manual_paste.rs").read_text(encoding="utf-8"), "send_paste_chord")
        wrapper = function((source / "protected_paste.rs").read_text(encoding="utf-8"), "run_with_preflight")
        for name, body in [("manual_paste.rs::send_paste_chord", chord),
                           ("protected_paste.rs::run_with_preflight", wrapper)]:
            manifest["source_functions"][name] = hashlib.sha256(body.encode()).hexdigest()
        safe_adapters = (f'\n#[path = "{keyboard}"] mod keyboard;\n'
                         f'#[path = "{protected}"] mod protected_policy;\n'
                         + SAFE_ADAPTERS.replace("// REAL_MANUAL_CHORD", chord.replace("std::thread::sleep", "crate::virtual_clock::sleep"))
                         .replace("// REAL_AUTOMATIC_WRAPPER", wrapper))
        if args.late_preflight:
            safe_adapters = safe_adapters.replace(
                "crate::protected_policy::transact_with_preflight(&mut provider, text, preflight, inject, cancelled, copy)",
                "crate::protected_policy::transact_with_retention(&mut provider, text, || { preflight()?; inject() }, cancelled, copy)")
    harness = (ADAPTERS + f'\n#[path = "{delivery}"] mod paste_delivery;\n'
               + 'use paste_delivery::{clipboard_submission, Submission};\n'
               + '\n'.join(functions["settings.rs"])
               + '\n'.join(functions["dictation_lifecycle.rs"])
               + '\nmod input { use super::*; pub struct EnigoState(pub Mutex<Enigo>);\n'
               + '\n'.join(functions["input.rs"]) + '\n}\n'
               + safe_adapters + (rdp_adapters(source) if not baseline else '') + '\n'.join(functions["clipboard.rs"])
               + '\n'.join(functions.get("actions.rs", []))
               + (TESTS if baseline else SAFE_TESTS))
    out = ROOT / "outputs/automatic-paste-016-tests"
    out.mkdir(parents=True, exist_ok=True)
    label = "legacy" if baseline else "current"
    manifest["source_modules"] = {
        filename: hashlib.sha256((source / filename).read_bytes()).hexdigest()
        for filename in ["paste_delivery.rs", "manual_paste_keyboard.rs", "protected_paste_policy.rs"]
    }
    rustfile = out / f"automatic_paste_{label}.rs"
    rustfile.write_text(harness, encoding="utf-8")
    (out / f"manifest_{label}.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    rustc = shutil.which("rustc") or str(Path.home() / ".cargo/bin/rustc.exe")
    executable = out / (f"automatic_paste_{label}.exe" if os.name == "nt" else f"automatic_paste_{label}")
    compile_result = subprocess.run([rustc, "--edition=2021", "--test", str(rustfile), "-o", str(executable)], check=False)
    if compile_result.returncode:
        return compile_result.returncode
    print("Synthetic adapters only; no OS clipboard or keyboard APIs. Clock advances instantly.", flush=True)
    test_filter = args.filter or ("characterize_" if args.characterize else ("regression_" if baseline else ""))
    result = subprocess.run([str(executable), test_filter], check=False, timeout=30, capture_output=True, text=True)
    print(result.stdout, end="")
    print(result.stderr, end="")
    suffix = "-characterize" if args.characterize else ""
    (out / f"{label}{suffix}-result.txt").write_text(result.stdout + result.stderr, encoding="utf-8")
    return result.returncode


if __name__ == "__main__":
    raise SystemExit(main())
