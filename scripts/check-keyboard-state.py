"""Synthetic high-function-key policy checks. No native input/window APIs run.

--legacy characterizes the real pre-observer aggregate and must fail the stale
F22 regression. --windows-rlib additionally compiles native adapter metadata only.
"""
from pathlib import Path
import argparse
import importlib.util
import os
import re
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]
TESTS = r'''
#[cfg(test)] mod regression {
use super::policy::*;
fn up() -> State { let mut s = State::new(); s.baseline(0); s }
#[test] fn observed_up_then_isolated_f22_latch_stops_blocking_after_two_synchronized_rounds() {
    let mut s = up(); assert_eq!(s.query(false, 1 << 9, 0, 100), Decision::Held);
    assert_eq!(s.query(false, 1 << 9, 0, 149), Decision::Held);
    assert_eq!(s.query(false, 1 << 9, 0, 150), Decision::Ready);
}
#[test] fn physical_f22_blocks_even_when_unbound_or_async_is_zero() {
    let mut s = up(); s.raw(1, Some(9), true);
    assert_eq!(s.query(false, 1 << 9, 0, 100), Decision::Held);
    assert_eq!(s.query(false, 0, 0, 200), Decision::Held);
}
#[test] fn configured_high_key_always_blocks_the_async_high() {
    let mut s = up(); s.query(false, 1 << 9, 1 << 9, 0);
    assert_eq!(s.query(false, 1 << 9, 1 << 9, 100), Decision::Held);
}
#[test] fn startup_high_without_an_up_anchor_remains_unknown() {
    let mut s = State::new();
    assert_eq!(s.query(false, 1 << 9, 0, 0), Decision::Unknown);
    assert_eq!(s.query(false, 1 << 9, 0, 10000), Decision::Unknown);
}
#[test] fn an_unknown_device_break_cannot_certify_another_preexisting_hold_is_up() {
    let mut s = State::new(); s.raw(1, Some(9), false);
    assert_eq!(s.query(false, 1 << 9, 0, 0), Decision::Unknown);
    assert_eq!(s.query(false, 1 << 9, 0, 100), Decision::Unknown);
}
#[test] fn reset_cannot_keep_a_stale_known_up_authorization() {
    let mut s = up(); s.query(false, 1 << 9, 0, 0); s.reset();
    assert_eq!(s.query(false, 1 << 9, 0, 200), Decision::Unknown);
}
#[test] fn raw_read_or_registration_failure_never_relaxes_a_high_key() {
    let mut s = up(); s.query(false, 1 << 9, 0, 0); s.fail();
    assert_eq!(s.query(false, 1 << 9, 0, 100), Decision::Unknown);
}
#[test] fn released_devices_do_not_consume_slots_for_a_later_real_physical_hold() {
    let mut s = up();
    for id in 1..=32 { s.raw(id, Some(9), true); s.raw(id, Some(9), false); }
    s.raw(33, Some(9), true);
    assert_eq!(s.query(false, 0, 0, 100), Decision::Held);
}
#[test] fn genuine_simultaneous_device_overflow_remains_unknown_after_tracked_keys_release() {
    let mut s=up(); for id in 1..=33 { s.raw(id,Some(9),true); }
    for id in 1..=32 { s.raw(id,Some(9),false); }
    assert_eq!(s.query(false,0,0,100),Decision::Unknown);
}
#[test] fn a_raw_read_failure_never_certifies_all_physical_keys_released() {
    let mut s=up(); s.fail(); assert_eq!(s.query(false,0,0,100),Decision::Unknown);
}
#[test] fn ordinary_key_and_modifier_protection_is_unchanged() {
    let mut s = up(); assert_eq!(s.query(true, 0, 0, 100), Decision::Held);
    assert_eq!(s.query(true, 1 << 9, 0, 200), Decision::Held);
}
#[test] fn raw_make_and_break_between_rounds_invalidates_the_old_candidate() {
    let mut s = up(); s.query(false, 1 << 9, 0, 0);
    s.raw(1, Some(9), true); s.raw(1, Some(9), false);
    assert_eq!(s.query(false, 1 << 9, 0, 100), Decision::Held);
    assert_eq!(s.query(false, 1 << 9, 0, 150), Decision::Ready);
}
#[test] fn even_an_unretained_ordinary_raw_event_invalidates_the_candidate() {
    let mut s = up(); s.query(false, 1 << 9, 0, 0); s.raw(1, None, true);
    assert_eq!(s.query(false, 1 << 9, 0, 100), Decision::Held);
}
#[test] fn two_devices_and_autorepeat_do_not_clear_another_physical_hold() {
    let mut s = up(); s.raw(1, Some(9), true); s.raw(1, Some(9), true);
    s.raw(2, Some(9), true); s.raw(1, Some(9), false);
    assert_eq!(s.query(false, 0, 0, 0), Decision::Held);
    s.remove_device(1); assert_eq!(s.query(false, 1 << 9, 0, 100), Decision::Held);
    s.raw(2, Some(9), false); s.baseline(0);
    assert_eq!(s.query(false, 0, 0, 200), Decision::Ready);
}
#[test] fn high_set_changes_restart_the_full_stability_window() {
    let mut s = up(); s.query(false, 1 << 9, 0, 0);
    assert_eq!(s.query(false, (1 << 9) | 1, 0, 100), Decision::Held);
    assert_eq!(s.query(false, (1 << 9) | 1, 0, 150), Decision::Ready);
}
#[test] fn only_high_function_bindings_are_classified() {
    assert_eq!(configured_mask(["alt_left+F22", "ctrl+f13", "F24"]), (1<<9)|1|(1<<11));
    assert_eq!(configured_mask(["f12", "shift+f8", "alt_left+super_left"]), 0);
}
#[test] fn arrival_removal_and_excess_devices_fail_conservatively() {
    let mut s = up(); s.query(false, 1<<9, 0, 0); s.remove_device(99);
    assert_eq!(s.query(false, 1<<9, 0, 100), Decision::Unknown);
    let mut s = up(); for id in 1..=33 { s.raw(id, Some(9), true); }
    assert_ne!(s.query(false, 1<<9, 0, 100), Decision::Ready);
}
}
'''

LEGACY_TEST = r'''
#![allow(non_snake_case)]
unsafe fn GetForegroundWindow() -> *mut std::ffi::c_void { 1usize as _ }
unsafe fn GetAsyncKeyState(vk: i32) -> i16 { if vk == 0x85 { i16::MIN } else { 0 } }
#[test] fn known_up_raw_f22_remains_up_but_async_latch_must_not_block_output() {
    // A trusted raw-up anchor and two synchronized 50ms rounds are part of the
    // new contract; the legacy aggregate cannot consume either observation.
    assert_eq!(any_keyboard_key_down(), Ok(false));
}
'''

ADAPTER_TESTS = r'''
#[cfg(test)] mod adapter {
#![allow(dead_code, non_snake_case)]
use std::cell::Cell;
use std::sync::{Mutex, atomic::{AtomicBool, AtomicU16, AtomicU32, AtomicU64, AtomicUsize, Ordering}};
use std::time::{Duration, Instant};
use std::collections::VecDeque;
const UNAVAILABLE: &str = "keyboard_unavailable";
const TOKEN_LIFETIME: Duration = Duration::from_millis(100);
const DRAIN_LIMIT: usize = 256;
const WM_INPUT_DEVICE_CHANGE: u32 = 254;
const WM_INPUT: u32 = 255;
const PM_REMOVE: u32 = 1;
const QS_RAWINPUT: u32 = 1024;
#[derive(Clone, Copy, PartialEq, Eq)] struct HWND(usize);
impl HWND { fn is_invalid(self) -> bool { self.0 == 0 } }
#[derive(Default)] struct MSG { message: u32 }
struct Bool(bool); impl Bool { fn as_bool(self) -> bool { self.0 } }
struct Context { failed: Cell<bool>, resets: Cell<u8> }
impl Context { fn fail(&self) { self.failed.set(true); } fn reset(&self) { self.resets.set(self.resets.get()+1); } }
thread_local! { static DESKTOP_RESET: Cell<bool> = const { Cell::new(false) }; }
static LOCK: Mutex<()> = Mutex::new(());
static RAW: Mutex<VecDeque<u32>> = Mutex::new(VecDeque::new());
static HINT: AtomicU32 = AtomicU32::new(0);
static DISPATCHED: AtomicUsize = AtomicUsize::new(0);
static PEEKS: AtomicUsize = AtomicUsize::new(0);
static ARRIVE_ON_STATUS: AtomicBool = AtomicBool::new(false);
static HIGH: AtomicU16 = AtomicU16::new(0);
static ORDINARY: AtomicBool = AtomicBool::new(false);
#[derive(Clone, Copy)] struct Flags(u32);
impl Flags { fn contains(self, other:Self)->bool { self.0 & other.0 == other.0 } }
impl std::ops::BitOr for Flags { type Output=Self; fn bitor(self,other:Self)->Self { Self(self.0|other.0) } }
const RIDEV_INPUTSINK:Flags=Flags(0x100);
const RIDEV_DEVNOTIFY:Flags=Flags(0x2000);
#[derive(Clone)] struct Registration { usUsagePage:u16, usUsage:u16, hwndTarget:HWND, dwFlags:Flags }
static REGISTRATIONS:Mutex<Vec<Registration>>=Mutex::new(Vec::new());
fn registrations()->Result<Vec<Registration>, &'static str> { Ok(REGISTRATIONS.lock().unwrap().clone()) }
struct Observer { generation: AtomicU64, alive: AtomicBool }
static OBS: Result<Observer, &'static str> = Ok(Observer { generation: AtomicU64::new(1), alive: AtomicBool::new(true) });
struct Global;
impl Global { fn get(&self) -> Option<&'static Result<Observer, &'static str>> { Some(&OBS) } }
static OBSERVER: Global = Global;
unsafe fn GetForegroundWindow() -> HWND { HWND(1) }
fn ordinary_down() -> bool { ORDINARY.load(Ordering::Relaxed) }
fn async_high() -> u16 { HIGH.load(Ordering::Relaxed) }
unsafe fn PeekMessageW(message: &mut MSG, _: Option<HWND>, minimum: u32, maximum: u32, _: u32) -> Bool {
    PEEKS.fetch_add(1,Ordering::Relaxed);
    assert_eq!((minimum,maximum), (254,255));
    if let Some(value) = RAW.lock().unwrap().pop_front() { message.message = value; Bool(true) } else { Bool(false) }
}
unsafe fn DispatchMessageW(_: &MSG) { DISPATCHED.fetch_add(1, Ordering::Relaxed); }
unsafe fn GetQueueStatus(_: u32) -> u32 {
    if ARRIVE_ON_STATUS.swap(false,Ordering::Relaxed) {
        RAW.lock().unwrap().push_back(WM_INPUT); return 1<<16;
    }
    HINT.load(Ordering::Relaxed)
}
// REAL_PREPARED
// REAL_FAST
// REAL_DRAIN
// REAL_OURS
fn reset() {
    RAW.lock().unwrap().clear(); HINT.store(0, Ordering::Relaxed); DISPATCHED.store(0, Ordering::Relaxed);
    PEEKS.store(0,Ordering::Relaxed); ARRIVE_ON_STATUS.store(false,Ordering::Relaxed);
    HIGH.store(0, Ordering::Relaxed); ORDINARY.store(false, Ordering::Relaxed);
    OBS.as_ref().unwrap().generation.store(1, Ordering::Relaxed);
    OBS.as_ref().unwrap().alive.store(true, Ordering::Relaxed);
    DESKTOP_RESET.with(|flag| flag.set(false));
    REGISTRATIONS.lock().unwrap().clear();
}
#[test] fn sent_query_must_drain_raw_and_device_events_before_authorizing() {
    let _lock = LOCK.lock().unwrap(); reset(); RAW.lock().unwrap().extend([254,255]);
    let c = Context { failed: Cell::new(false), resets: Cell::new(0) };
    assert!(drain_raw(HWND(1),&c)); assert_eq!(DISPATCHED.load(Ordering::Relaxed),2); assert!(!c.failed.get());
}
#[test] fn draining_limit_or_current_raw_queue_bit_never_means_synchronized() {
    let _lock = LOCK.lock().unwrap(); reset(); RAW.lock().unwrap().extend(std::iter::repeat_n(255,DRAIN_LIMIT+1));
    let c = Context { failed: Cell::new(false), resets: Cell::new(0) };
    assert!(!drain_raw(HWND(1),&c)); assert!(c.failed.get());
    RAW.lock().unwrap().clear(); HINT.store(1<<16,Ordering::Relaxed);
    assert!(!drain_raw(HWND(1),&c));
}
#[test] fn queue_changed_low_word_is_not_the_current_pending_bit() {
    let _lock = LOCK.lock().unwrap(); reset(); HINT.store(1,Ordering::Relaxed);
    let c = Context { failed: Cell::new(false), resets: Cell::new(0) };
    assert!(drain_raw(HWND(1),&c)); assert!(!c.failed.get());
}
#[test] fn raw_arriving_between_empty_peek_and_status_is_drained_in_the_next_round() {
    let _lock=LOCK.lock().unwrap();reset();ARRIVE_ON_STATUS.store(true,Ordering::Relaxed);
    let c=Context{failed:Cell::new(false),resets:Cell::new(0)};
    assert!(drain_raw(HWND(1),&c));assert_eq!(DISPATCHED.load(Ordering::Relaxed),1);
    assert!(!c.failed.get());
}
#[test] fn a_persistent_pending_hint_without_a_packet_still_exhausts_the_bounded_budget() {
    let _lock=LOCK.lock().unwrap();reset();HINT.store(1<<16,Ordering::Relaxed);
    let c=Context{failed:Cell::new(false),resets:Cell::new(0)};
    assert!(!drain_raw(HWND(1),&c));assert!(c.failed.get());
    assert_eq!(PEEKS.load(Ordering::Relaxed),256);
}
#[test] fn desktop_reset_is_applied_after_the_explicit_drain() {
    let _lock = LOCK.lock().unwrap(); reset(); DESKTOP_RESET.with(|flag| flag.set(true));
    let c = Context { failed: Cell::new(false), resets: Cell::new(0) };
    assert!(drain_raw(HWND(1),&c)); assert_eq!(c.resets.get(),1);
}
#[test] fn normalized_registration_readback_keeps_the_requested_sink_owner_valid() {
    let _lock=LOCK.lock().unwrap(); reset();
    // The native registration requests 0x2100, but Windows reads back 0x100.
    REGISTRATIONS.lock().unwrap().push(Registration {usUsagePage:1,usUsage:6,hwndTarget:HWND(1),dwFlags:Flags(0x100)});
    assert!(ours(HWND(1)),"DEVNOTIFY need not be returned by GetRegisteredRawInputDevices");
}
#[test] fn registration_normalization_never_adopts_a_foreign_receiver() {
    let _lock=LOCK.lock().unwrap(); reset();
    REGISTRATIONS.lock().unwrap().push(Registration {usUsagePage:1,usUsage:6,hwndTarget:HWND(2),dwFlags:Flags(0x100)});
    assert!(!ours(HWND(1)));
}
#[test] fn registration_without_input_sink_never_authorizes_background_observation() {
    let _lock=LOCK.lock().unwrap(); reset();
    REGISTRATIONS.lock().unwrap().push(Registration {usUsagePage:1,usUsage:6,hwndTarget:HWND(1),dwFlags:Flags(0x2000)});
    assert!(!ours(HWND(1)));
}
#[test] fn final_fast_check_rejects_new_physical_generation_new_binding_and_ordinary_key() {
    let _lock = LOCK.lock().unwrap(); reset(); HIGH.store(1<<9,Ordering::Relaxed);
    let p = Prepared { generation:1,high:1<<9,at:Instant::now() };
    assert_eq!(fast_check(&p,0),Ok(false));
    assert_eq!(fast_check(&p,1<<9),Ok(true));
    OBS.as_ref().unwrap().generation.store(2,Ordering::Relaxed);
    assert_eq!(fast_check(&p,0),Ok(true));
    HIGH.store(0,Ordering::Relaxed);
    let clear_token = Prepared { generation:1,high:0,at:Instant::now() };
    assert_eq!(fast_check(&clear_token,0),Ok(true), "physical raw down invalidates proof even after a software key-up made async zero");
    OBS.as_ref().unwrap().generation.store(1,Ordering::Relaxed); ORDINARY.store(true,Ordering::Relaxed);
    assert_eq!(fast_check(&p,0),Ok(true));
}
#[test] fn final_fast_check_rejects_expired_token_stopped_thread_or_changed_high_set() {
    let _lock = LOCK.lock().unwrap(); reset();
    let mut p = Prepared { generation:1,high:0,at:Instant::now()-Duration::from_millis(101) };
    assert_eq!(fast_check(&p,0),Ok(true)); p.at=Instant::now();
    HIGH.store(1<<9,Ordering::Relaxed); assert_eq!(fast_check(&p,0),Ok(true));
    HIGH.store(0,Ordering::Relaxed); OBS.as_ref().unwrap().alive.store(false,Ordering::Relaxed);
    assert_eq!(fast_check(&p,0),Ok(true));
}
#[test] fn missing_observer_fallback_never_relaxes_any_high_key() {
    let _lock = LOCK.lock().unwrap(); reset();
    let p = Prepared { generation:0,high:0,at:Instant::now() };
    assert_eq!(fast_check(&p,0),Ok(false)); HIGH.store(1<<9,Ordering::Relaxed);
    assert_eq!(fast_check(&p,0),Ok(true));
}
}
'''

GRACE_TESTS = r'''
#[cfg(test)] mod grace {
#![allow(private_interfaces)]
use std::cell::Cell;
use std::time::Duration;
thread_local! { static CLOCK: Cell<u64> = const { Cell::new(0) }; static RELEASE: Cell<u64> = const { Cell::new(0) }; }
#[derive(Clone,Copy,PartialEq,Eq,PartialOrd,Ord)] struct Instant(u64);
impl Instant { fn now() -> Self { Self(CLOCK.with(Cell::get)) } }
impl std::ops::Add<Duration> for Instant { type Output=Self; fn add(self,d:Duration)->Self { Self(self.0+d.as_millis() as u64) } }
fn sleep(d:Duration) { CLOCK.with(|c| c.set(c.get()+d.as_millis() as u64)); }
struct AppHandle;
mod settings { pub(super) fn get_settings(_: &super::AppHandle) {} }
mod keyboard { pub const CANCELLED: &str="manual_paste_cancelled"; }
const RELEASE_POLL: Duration=Duration::from_millis(10);
fn any_keyboard_key_down_for_settings(_: &()) -> Result<bool,&'static str> { Ok(CLOCK.with(Cell::get)<RELEASE.with(Cell::get)) }
// REAL_RELEASE_WAIT
// REAL_AUTOMATIC_WAIT
#[test] fn actual_auto_grace_allows_release_before_borrowing_clipboard() {
    CLOCK.with(|c|c.set(0)); RELEASE.with(|c|c.set(60));
    assert_eq!(wait_before_automatic(&AppHandle,||false),Ok(()));
    assert_eq!(CLOCK.with(Cell::get),60);
}
#[test] fn actual_auto_grace_has_a_400ms_deadline_without_keys_or_clipboard() {
    CLOCK.with(|c|c.set(0)); RELEASE.with(|c|c.set(1000));
    assert_eq!(wait_before_automatic(&AppHandle,||false),Err("keys_held".into()));
    assert_eq!(CLOCK.with(Cell::get),400);
}
#[test] fn actual_auto_grace_cancel_does_not_wait_until_the_deadline() {
    CLOCK.with(|c|c.set(0)); RELEASE.with(|c|c.set(1000));
    assert_eq!(wait_before_automatic(&AppHandle,||CLOCK.with(Cell::get)>=20),Err("manual_paste_cancelled".into()));
    assert_eq!(CLOCK.with(Cell::get),20);
}
}
'''

REQUEST_TESTS = r'''
#[cfg(test)] mod request_branch {
#![allow(non_snake_case)]
use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicU64,Ordering};
use std::time::Duration;
use super::policy::{State,Decision};
const UNAVAILABLE:&str="keyboard_unavailable";
const QS_RAWINPUT:u32=1024;
thread_local! {
 static NOW:Cell<u64>=const {Cell::new(0)};
 static HIGH:Cell<u16>=const {Cell::new(0)};
 static PENDING:Cell<bool>=const {Cell::new(false)};
 static PENDING_ON_SCAN:Cell<bool>=const {Cell::new(false)};
 static CHANGE_ON_SCAN:Cell<bool>=const {Cell::new(false)};
 static ACTIVE:Cell<*const Context>=const {Cell::new(std::ptr::null())};
}
#[derive(Clone,Copy,PartialEq,Eq,PartialOrd,Ord)] struct Instant(u64);
impl Instant { fn now()->Self {Self(NOW.with(Cell::get))} fn elapsed(self)->Duration {Duration::from_millis(NOW.with(Cell::get).saturating_sub(self.0))} }
#[derive(Clone,Copy)] struct HWND;
struct Prepared { generation:u64, high:u16, at:Instant }
struct Request { deadline:Instant, bound:u16 }
struct Context { state:RefCell<State>, generation:AtomicU64 }
impl Context {
 fn new(anchored:bool)->Self {let mut state=State::new();if anchored {state.baseline(0)} Self{state:RefCell::new(state),generation:AtomicU64::new(1)}}
 fn fail(&self) {self.generation.fetch_add(1,Ordering::AcqRel);self.state.borrow_mut().fail();}
 fn revoke(&self) {self.generation.fetch_add(1,Ordering::AcqRel); /* REAL_STATE_REVOKE */ }
}
fn ours(_:HWND)->bool {true}
fn drain_raw(_:HWND,context:&Context)->bool {
 if PENDING.with(|flag|flag.replace(false)) {
  context.generation.fetch_add(1,Ordering::AcqRel);context.state.borrow_mut().raw(1,None,true);
 }
 true
}
fn async_high()->u16 {HIGH.with(Cell::get)}
fn ordinary_down()->bool {
 if PENDING_ON_SCAN.with(|flag|flag.replace(false)) {PENDING.with(|flag|flag.set(true));}
 if CHANGE_ON_SCAN.with(|flag|flag.replace(false)) {ACTIVE.with(|ptr|unsafe {(&*ptr.get()).generation.fetch_add(1,Ordering::AcqRel);});}
 false
}
unsafe fn GetQueueStatus(_:u32)->u32 {if PENDING.with(Cell::get) {1<<16} else {0}}
fn process(context:&Context,deadline:u64)->Result<Prepared,&'static str> {
 let request=Request{deadline:Instant(deadline),bound:0};let hwnd=HWND;let began=Instant(0);
 ACTIVE.with(|ptr|ptr.set(context as *const _));
 // REAL_REQUEST_BRANCH
 result
}
#[test] fn a_late_request_is_rejected_but_the_next_fresh_all_up_request_recovers() {
 let context=Context::new(true);NOW.with(|c|c.set(100));
 assert_eq!(process(&context,99).err(),Some(UNAVAILABLE));
 NOW.with(|c|c.set(101));assert!(process(&context,200).is_ok());
}
#[test] fn pending_raw_rejects_this_round_but_fresh_drain_recovers() {
 let context=Context::new(true);NOW.with(|c|c.set(100));PENDING_ON_SCAN.with(|c|c.set(true));
 assert_eq!(process(&context,200).err(),Some(UNAVAILABLE));
 NOW.with(|c|c.set(101));assert!(process(&context,200).is_ok());
}
#[test] fn generation_change_rejects_only_that_request() {
 let context=Context::new(true);NOW.with(|c|c.set(100));CHANGE_ON_SCAN.with(|c|c.set(true));
 assert_eq!(process(&context,200).err(),Some(UNAVAILABLE));
 NOW.with(|c|c.set(101));assert!(process(&context,200).is_ok());
}
#[test] fn deadline_after_scan_is_rejected_without_permanent_failure() {
 let context=Context::new(true);NOW.with(|c|c.set(100));
 // The source expression's post-scan deadline branch is also exercised by
 // a scripted clock change in ordinary_down (installed by the generator).
 AFTER_SCAN_DEADLINE.with(|flag|flag.set(true));
 assert_eq!(process(&context,120).err(),Some(UNAVAILABLE));
 NOW.with(|c|c.set(201));assert!(process(&context,300).is_ok());
}
#[test] fn virtual_f22_high_retains_its_up_anchor_across_a_soft_timeout() {
 let context=Context::new(true);HIGH.with(|c|c.set(1<<9));NOW.with(|c|c.set(100));
 assert_eq!(process(&context,99).err(),Some(UNAVAILABLE));
 NOW.with(|c|c.set(200));assert_eq!(process(&context,400).err(),Some("keys_held"));
 NOW.with(|c|c.set(250));assert!(process(&context,400).is_ok());
}
#[test] fn startup_unknown_cannot_get_an_up_anchor_from_soft_revocation() {
 let context=Context::new(false);HIGH.with(|c|c.set(1<<9));NOW.with(|c|c.set(100));
 assert_eq!(process(&context,99).err(),Some(UNAVAILABLE));
 NOW.with(|c|c.set(200));assert_eq!(process(&context,400).err(),Some("keys_held"));
 NOW.with(|c|c.set(300));assert_eq!(process(&context,400).err(),Some("keys_held"));
}
#[test] fn physical_down_survives_soft_revocation_even_if_async_is_zero() {
 let context=Context::new(true);context.state.borrow_mut().raw(1,Some(9),true);NOW.with(|c|c.set(100));
 assert_eq!(process(&context,99).err(),Some(UNAVAILABLE));
 NOW.with(|c|c.set(200));assert_eq!(process(&context,400).err(),Some("keys_held"));
}
#[test] fn a_hard_read_failure_cannot_be_repaired_by_soft_revocation() {
 let context=Context::new(true);context.fail();NOW.with(|c|c.set(100));
 assert_eq!(process(&context,99).err(),Some(UNAVAILABLE));
 NOW.with(|c|c.set(200));assert_eq!(process(&context,400).err(),Some(UNAVAILABLE));
 assert_eq!(context.state.borrow_mut().query(false,0,0,300),Decision::Unknown);
}
#[test] fn a_real_capacity_failure_stays_unknown_after_soft_revocation() {
 let context=Context::new(true);
 for device in 1..=33 {context.state.borrow_mut().raw(device,Some(9),true);}
 for device in 1..=32 {context.state.borrow_mut().raw(device,Some(9),false);}
 NOW.with(|c|c.set(100));assert_eq!(process(&context,99).err(),Some(UNAVAILABLE));
 NOW.with(|c|c.set(200));assert_eq!(process(&context,400).err(),Some(UNAVAILABLE));
 assert_eq!(context.state.borrow_mut().query(false,0,0,300),Decision::Unknown);
}
fn accept_reply(received_id:u64,id:u64,deadline:Instant)->Result<(), &'static str> {
 // REAL_REPLY_GUARD
 Ok(())
}
#[test] fn the_caller_also_rejects_a_late_or_mismatched_reply() {
 NOW.with(|c|c.set(41));assert_eq!(accept_reply(1,1,Instant(40)),Err(UNAVAILABLE));
 NOW.with(|c|c.set(39));assert_eq!(accept_reply(1,1,Instant(40)),Ok(()));
 assert_eq!(accept_reply(2,1,Instant(40)),Err(UNAVAILABLE));
}
thread_local! {static AFTER_SCAN_DEADLINE:Cell<bool>=const {Cell::new(false)};}
}
'''

def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--legacy", action="store_true")
    p.add_argument("--windows-rlib", type=Path)
    args = p.parse_args()
    out = ROOT / "outputs/020-keyboard-state-tests"
    out.mkdir(exist_ok=True)
    rustc = shutil.which("rustc") or str(Path.home() / ".cargo/bin/rustc.exe")
    if args.legacy:
        spec = importlib.util.spec_from_file_location("paste_check", ROOT / "scripts/check-automatic-paste.py")
        m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)
        source = subprocess.check_output(["git", "show", "v0.1.9:desktop/src-tauri/src/manual_paste.rs"], cwd=ROOT, text=True, encoding="utf-8")
        body = m.function(source, "any_keyboard_key_down")
        body = re.sub(r'    #\[link\(name = "user32"\)\]\n    extern "system" \{.*?\n    \}\n', "", body, count=1, flags=re.S)
        text = LEGACY_TEST + body
    else:
        policy = (ROOT / "desktop/src-tauri/src/keyboard_state_policy.rs").as_posix()
        text = f'#![allow(dead_code)]\n#[path = "{policy}"] mod policy;\n' + TESTS
        spec = importlib.util.spec_from_file_location("paste_check", ROOT / "scripts/check-automatic-paste.py")
        m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)
        native = (ROOT / "desktop/src-tauri/src/keyboard_state_windows.rs").read_text(encoding="utf-8")
        for forbidden in ["SendInput", "keybd_event", "ToUnicode", "GetKeyNameText"]:
            assert forbidden not in native
        begin = native.index("pub(crate) struct Prepared {")
        end = native.index("\n}",begin)+2
        text += ADAPTER_TESTS.replace("// REAL_PREPARED",native[begin:end]).replace("// REAL_FAST",m.function(native,"fast_check")).replace("// REAL_DRAIN",m.function(native,"drain_raw")).replace("// REAL_OURS",m.function(native,"ours"))
        release = (ROOT / "desktop/src-tauri/src/manual_paste_policy.rs").read_text(encoding="utf-8")
        release = release[release.index("#[derive(Debug, PartialEq, Eq)]"):release.rfind("#[derive", 0, release.index("pub enum ManualOutcome"))]
        manual = (ROOT / "desktop/src-tauri/src/manual_paste.rs").read_text(encoding="utf-8")
        wait = m.function(manual,"wait_before_automatic").replace("std::thread::sleep", "sleep")
        text += GRACE_TESTS.replace("// REAL_RELEASE_WAIT",release).replace("// REAL_AUTOMATIC_WAIT",wait)
        branch_start=native.index("let result = if Instant::now() >= request.deadline")
        branch_end=native.index("let _ = request.reply.send",branch_start)
        branch=native[branch_start:branch_end].strip()
        revoke="self.state.borrow_mut().revoke();" if "fn revoke(" in Path(policy).read_text(encoding="utf-8") else ""
        request_tests=REQUEST_TESTS.replace("// REAL_REQUEST_BRANCH",branch).replace("/* REAL_STATE_REVOKE */",revoke)
        reply_start=native.index("if received_id != id")
        reply_end=native.index("\n    result",reply_start)
        request_tests=request_tests.replace("// REAL_REPLY_GUARD",native[reply_start:reply_end])
        request_tests=request_tests.replace("fn ordinary_down()->bool {", "fn ordinary_down()->bool { if AFTER_SCAN_DEADLINE.with(|flag|flag.replace(false)) {NOW.with(|clock|clock.set(200));}")
        text+=request_tests
    label = "legacy" if args.legacy else "current"
    # Expected red assertions must not poison the fixture's serial-test lock
    # and create unrelated failures in subsequent adapter cases.
    text = text.replace("LOCK.lock().unwrap()", "LOCK.lock().unwrap_or_else(|error| error.into_inner())")
    sourcefile, executable = out / f"{label}.rs", out / f"{label}.exe"
    sourcefile.write_text(text, encoding="utf-8")
    subprocess.run([rustc, "--edition=2021", "--test", str(sourcefile), "-o", str(executable)], check=True)
    result = subprocess.run([str(executable)], capture_output=True, text=True)
    print(result.stdout + result.stderr)
    (out / f"{label}.txt").write_text(result.stdout + result.stderr, encoding="utf-8")
    if result.returncode or not args.windows_rlib:
        return result.returncode
    rlib = args.windows_rlib.resolve(strict=True)
    native = (ROOT / "desktop/src-tauri/src/keyboard_state_windows.rs").as_posix()
    policy = (ROOT / "desktop/src-tauri/src/keyboard_state_policy.rs").as_posix()
    metadata = out / "native_metadata.rs"
    metadata.write_text(f'#![allow(dead_code)]\n#[path="{policy}"] mod keyboard_state_policy;\n#[path="{native}"] mod keyboard_state_windows;\nfn main() {{}}', encoding="utf-8")
    subprocess.run([rustc, "--edition=2021", "--emit=metadata", str(metadata), "--extern", f"windows={rlib}", "-L", f"dependency={rlib.parent}", "-o", str(out / "native.rmeta")], check=True)
    print("Native keyboard observer metadata compiled; no native APIs executed.")
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
