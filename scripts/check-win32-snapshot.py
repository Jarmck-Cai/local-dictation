"""Run exact Win32 clipboard snapshot bodies against synthetic in-memory APIs.

No native clipboard/window/keyboard functions are linked or called. The synthetic
adapter can emulate on-demand format synthesis, including fresh handles for an
unchanged format. --baseline reads v0.1.6 to preserve the original red regression.

Explicit --native-snapshot uses only clipboard read/lock APIs and backs up into
process memory; it never publishes, sends keys, or writes clipboard data to disk.
--build-native-only prepares that probe without running any native clipboard API.
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
extract_function = module.function

ADAPTERS = r'''
#![allow(dead_code, non_snake_case, non_camel_case_types, unused_variables)]
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::mem::size_of;
const BACKUP_FAILED: &str = "manual_paste_backup_failed";
const UNSUPPORTED_FORMAT: &str = "manual_paste_unsupported_format";
const BITMAP_FORMAT: u32 = 2;
const ERROR_SUCCESS: u32 = 0;
const IMAGE_BITMAP: u32 = 0;
const LR_CREATEDIBSECTION: u32 = 0;
const GMEM_MOVEABLE: u32 = 2;
const GMEM_ZEROINIT: u32 = 64;
use policy::{Budget, MAX_FORMATS};
#[derive(Clone, Copy, Debug, PartialEq, Eq)] struct HANDLE(usize);
#[derive(Clone, Copy)] struct HGLOBAL(usize);
#[derive(Clone, Copy)] struct HGDIOBJ(usize);
#[derive(Default)] struct BITMAP { bmWidth: i32, bmHeight: i32, bmWidthBytes: i32, bmPlanes: u16, bmBitsPixel: u16 }
struct Bitmap(HANDLE);
enum SavedData { Global(Global), Bitmap(Bitmap) }
struct Saved { format: u32, data: Option<SavedData>, source: HANDLE }
struct Global(HGLOBAL);
impl Global {
    // REAL_GLOBAL_ALLOCATE
}
struct MemoryLock { ptr: *mut u8 }
impl MemoryLock {
    unsafe fn new(handle: HGLOBAL) -> Result<Self, &'static str> {
        SYSTEM.with(|s| {
            let mut s = s.borrow_mut();
            if s.unreadable_handle == Some(handle.0) { return Err(BACKUP_FAILED); }
            s.memory.get_mut(&handle.0).map(|bytes| Self { ptr: bytes.as_mut_ptr() }).ok_or(BACKUP_FAILED)
        })
    }
}
#[derive(Default)]
struct System {
    formats: Vec<u32>, memory: HashMap<usize, Box<[u8]>>, entries: HashMap<u32, usize>,
    gets: HashMap<u32, usize>, next: usize, last_error: u32, fresh_format: Option<u32>,
    trace: Vec<String>,
    changed_bytes: bool, changed_same_handle: bool,
    copy_padding: usize,
    allocations: Vec<(usize, usize)>,
    fresh_extra_size: usize, add_format_on_verify: bool,
    unreadable_format: Option<u32>, unreadable_handle: Option<usize>,
    reported_sizes: HashMap<usize, usize>, allocation_failure: bool,
    bitmap_valid: bool,
}
impl System {
    fn allocate(&mut self, bytes: Vec<u8>) -> usize {
        self.next += 1; self.memory.insert(self.next, bytes.into_boxed_slice()); self.next
    }
}
thread_local! { static SYSTEM: RefCell<System> = RefCell::new(System::default()); }
unsafe fn SetLastError(error: u32) { SYSTEM.with(|s| s.borrow_mut().last_error = error); }
unsafe fn GetLastError() -> u32 { SYSTEM.with(|s| s.borrow().last_error) }
unsafe fn EnumClipboardFormats(previous: u32) -> u32 {
    SYSTEM.with(|s| { let s = s.borrow();
        let next = if previous == 0 { 0 } else { s.formats.iter().position(|&f| f == previous).unwrap() + 1 };
        s.formats.get(next).copied().unwrap_or(0)
    })
}
unsafe fn GetClipboardData(format: u32) -> Result<HANDLE, ()> {
    SYSTEM.with(|s| { let mut s = s.borrow_mut();
        if s.unreadable_format == Some(format) { return Err(()); }
        let count = s.gets.entry(format).or_default(); *count += 1; let count = *count;
        let original = *s.entries.get(&format).ok_or(())?;
        if s.changed_same_handle && s.fresh_format == Some(format) && count > 1 {
            s.memory.get_mut(&original).unwrap()[0] = b'X';
        }
        let handle = if s.fresh_format == Some(format) && count > 1 {
            if s.changed_same_handle { original } else {
                let mut bytes = s.memory[&original].to_vec();
                if s.changed_bytes { bytes[0] = b'X'; }
                let expanded = bytes.len() + s.fresh_extra_size; bytes.resize(expanded, 0);
                s.allocate(bytes)
            }
        } else { original };
        s.trace.push(format!("get format={} call={} handle={}", format, count, handle));
        if s.add_format_on_verify && format == 7 && count == 2 { s.formats.push(0xc012); }
        Ok(HANDLE(handle))
    })
}
unsafe fn GlobalSize(handle: HGLOBAL) -> usize {
    SYSTEM.with(|s| { let s = s.borrow(); s.reported_sizes.get(&handle.0).copied()
        .unwrap_or_else(|| s.memory.get(&handle.0).map_or(0, |bytes| bytes.len())) })
}
unsafe fn GlobalAlloc(flags: u32, bytes: usize) -> Result<HGLOBAL, ()> {
    SYSTEM.with(|s| { let mut s = s.borrow_mut(); let padding = s.copy_padding;
        if s.allocation_failure { return Err(()); }
        let initial = if flags & GMEM_ZEROINIT != 0 { 0 } else { 0xa5 };
        let handle = s.allocate(vec![initial; bytes + padding]); s.allocations.push((handle, bytes)); Ok(HGLOBAL(handle)) })
}
unsafe fn GetObjectW(_: HGDIOBJ, _: i32, out: Option<*mut std::ffi::c_void>) -> i32 {
    if !SYSTEM.with(|s| s.borrow().bitmap_valid) { return 0; }
    *out.unwrap().cast::<BITMAP>() = BITMAP { bmWidth: 2, bmHeight: 2, bmWidthBytes: 8, bmPlanes: 1, bmBitsPixel: 32 };
    size_of::<BITMAP>() as i32
}
unsafe fn CopyImage(_: HANDLE, _: u32, _: i32, _: i32, _: u32) -> Result<HANDLE, ()> {
    Ok(HANDLE(SYSTEM.with(|s| s.borrow_mut().allocate(vec![0; 16]))))
}
fn safe_global_format(format: u32) -> bool {
    let name = match format { 0xc001 => Some("HTML Format"), 0xc002 => Some("Rich Text Format"),
        0xc003 => Some("PNG"), 0xc004 => Some("CanIncludeInClipboardHistory"), _ => None };
    policy::safe_global_format(format, name)
}
'''

TESTS = r'''
#[cfg(test)]
mod tests {
    use super::*;
    fn text_clipboard(fresh_format: Option<u32>) {
        SYSTEM.with(|s| { let mut s = s.borrow_mut(); *s = System::default();
            s.formats = vec![13, 16, 1, 7]; s.fresh_format = fresh_format;
            for format in [13, 16, 1, 7] {
                let bytes = if format == 16 { vec![9, 4, 0, 0] } else { vec![b'S', b'Y', b'N', 0] };
                let handle = s.allocate(bytes); s.entries.insert(format, handle);
            }
        });
    }
    #[test]
    fn original_snapshot_accepts_stable_synthetic_global_handles() {
        text_clipboard(None);
        assert!(snapshot().is_ok());
    }
    #[test]
    fn unchanged_synthesized_text_with_a_fresh_handle_is_still_a_complete_snapshot() {
        for format in [1, 7] {
            text_clipboard(Some(format));
            let result = snapshot();
            SYSTEM.with(|s| eprintln!("synthetic API trace: {:?}", s.borrow().trace));
            assert!(result.is_ok(), "same format/bytes under one held clipboard lock must not fail only because format {} gets a fresh handle: {:?}", format, result.err());
        }
    }
    #[test]
    fn unknown_format_is_skipped_without_requesting_its_data() {
        text_clipboard(None);
        SYSTEM.with(|s| s.borrow_mut().formats.push(0xc012));
        let saved = snapshot().expect("unknown format must not block supported text backup");
        assert_eq!(saved.len(), 4);
        SYSTEM.with(|s| assert!(!s.borrow().gets.contains_key(&0xc012)));
    }
    #[test]
    fn entirely_unknown_original_allows_an_empty_backup() {
        text_clipboard(None);
        SYSTEM.with(|s| s.borrow_mut().formats = vec![0xc012]);
        assert!(snapshot().unwrap().is_empty());
        SYSTEM.with(|s| assert!(s.borrow().gets.is_empty()));
    }
    #[test]
    fn unreadable_known_format_does_not_discard_other_saved_payloads() {
        text_clipboard(None);
        SYSTEM.with(|s| s.borrow_mut().unreadable_format = Some(1));
        let saved = snapshot().unwrap();
        assert_eq!(saved.iter().map(|s| s.format).collect::<Vec<_>>(), vec![13, 16, 7]);
    }
    #[test]
    fn oversized_format_does_not_consume_the_budget_for_later_text() {
        text_clipboard(None);
        SYSTEM.with(|s| { let mut s = s.borrow_mut(); let handle = s.entries[&13];
            s.reported_sizes.insert(handle, policy::MAX_BYTES + 1); });
        let saved = snapshot().unwrap();
        assert_eq!(saved.iter().map(|s| s.format).collect::<Vec<_>>(), vec![16, 1, 7]);
    }
    #[test]
    fn more_than_64_formats_is_a_backup_budget_not_a_global_rejection() {
        text_clipboard(None);
        SYSTEM.with(|s| { let mut s = s.borrow_mut(); s.formats.extend(0xc100..0xc160); });
        assert_eq!(enumerate().unwrap().len(), 100);
        assert_eq!(snapshot().unwrap().len(), 4);
    }
    #[test]
    fn common_registered_payloads_and_hdrop_are_still_saved() {
        text_clipboard(None);
        SYSTEM.with(|s| { let mut s = s.borrow_mut();
            for format in [8, 15, 17, 0xc001, 0xc002, 0xc003, 0xc004] {
                s.formats.push(format); let handle = s.allocate(vec![1, 2, 3, 0]);
                s.entries.insert(format, handle);
            }
        });
        assert_eq!(snapshot().unwrap().len(), 11);
    }
    #[test]
    fn bitmap_uses_the_gdi_copy_path_without_assuming_global_memory() {
        text_clipboard(None);
        SYSTEM.with(|s| { let mut s = s.borrow_mut(); s.bitmap_valid = true;
            s.formats.push(2); s.entries.insert(2, 9999); });
        let saved = snapshot().unwrap();
        assert_eq!(saved.len(), 5);
        assert!(matches!(saved.last().unwrap().data, Some(SavedData::Bitmap(_))));
    }
    #[test]
    fn invalid_bitmap_is_skipped_without_losing_supported_text() {
        text_clipboard(None);
        SYSTEM.with(|s| { let mut s = s.borrow_mut(); s.formats.push(2); s.entries.insert(2, 9999); });
        assert_eq!(snapshot().unwrap().len(), 4);
    }
    #[test]
    fn global_lock_failure_is_skipped_without_losing_other_formats() {
        text_clipboard(None);
        SYSTEM.with(|s| { let mut s = s.borrow_mut(); s.unreadable_handle = Some(s.entries[&13]); });
        assert_eq!(snapshot().unwrap().iter().map(|s| s.format).collect::<Vec<_>>(), vec![16, 1, 7]);
    }
    #[test]
    fn zero_sized_or_unallocatable_original_can_yield_an_empty_backup() {
        text_clipboard(None);
        SYSTEM.with(|s| s.borrow_mut().allocation_failure = true);
        assert!(snapshot().unwrap().is_empty());
        text_clipboard(None);
        SYSTEM.with(|s| { let mut s = s.borrow_mut(); let handle = s.entries[&13];
            s.reported_sizes.insert(handle, 0); });
        assert_eq!(snapshot().unwrap().len(), 3);
    }
    #[test]
    fn duplicate_format_enumeration_remains_a_hard_failure() {
        text_clipboard(None); SYSTEM.with(|s| s.borrow_mut().formats = vec![13, 13]);
        assert_eq!(enumerate().err(), Some(BACKUP_FAILED));
    }
    #[test]
    fn a_fresh_handle_with_changed_bytes_still_refuses_backup() {
        text_clipboard(Some(1));
        SYSTEM.with(|s| s.borrow_mut().changed_bytes = true);
        assert_eq!(snapshot().err(), Some(BACKUP_FAILED));
    }
    #[test]
    fn unchanged_handle_with_changed_bytes_is_not_a_consistent_snapshot() {
        text_clipboard(Some(1));
        SYSTEM.with(|s| s.borrow_mut().changed_same_handle = true);
        assert_eq!(snapshot().err(), Some(BACKUP_FAILED), "handle identity alone does not prove saved bytes are unchanged");
    }
    #[test]
    fn allocator_rounding_of_the_backup_is_not_a_source_content_change() {
        text_clipboard(None);
        SYSTEM.with(|s| s.borrow_mut().copy_padding = 8);
        assert!(snapshot().is_ok(), "GlobalAlloc may allocate more capacity than the requested snapshot bytes");
    }
    #[test]
    fn restoring_a_rounded_backup_never_exposes_uninitialized_padding() {
        text_clipboard(None);
        SYSTEM.with(|s| s.borrow_mut().copy_padding = 8);
        assert!(snapshot().is_ok());
        SYSTEM.with(|s| { let s = s.borrow(); assert!(!s.allocations.is_empty());
            for &(handle, copied) in &s.allocations {
                assert!(s.memory[&handle][copied..].iter().all(|&byte| byte == 0),
                    "all bytes beyond the copied source length must be zero before a backup handle can be restored");
            }
        });
    }
    #[test]
    fn changed_source_allocation_size_is_still_a_conservative_refusal() {
        text_clipboard(Some(1));
        SYSTEM.with(|s| s.borrow_mut().fresh_extra_size = 8);
        assert_eq!(snapshot().err(), Some(BACKUP_FAILED));
    }
    #[test]
    fn a_format_added_during_final_verification_never_creates_a_partial_snapshot() {
        text_clipboard(None);
        SYSTEM.with(|s| s.borrow_mut().add_format_on_verify = true);
        assert_eq!(snapshot().err(), Some(BACKUP_FAILED));
    }
}
'''

NATIVE_ADAPTERS = r'''
#![allow(dead_code)]
use std::collections::HashSet;
use std::mem::size_of;
use windows::Win32::Foundation::{GetLastError, GlobalFree, SetLastError, ERROR_SUCCESS, HANDLE, HGLOBAL};
use windows::Win32::Graphics::Gdi::{DeleteObject, GetObjectW, BITMAP, HGDIOBJ};
use windows::Win32::System::DataExchange::{CloseClipboard, EnumClipboardFormats, GetClipboardData, GetClipboardFormatNameW, OpenClipboard};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE, GMEM_ZEROINIT};
use windows::Win32::UI::WindowsAndMessaging::{CopyImage, IMAGE_BITMAP, LR_CREATEDIBSECTION};
use policy::{Budget, MAX_FORMATS};
const BACKUP_FAILED: &str = "manual_paste_backup_failed";
const UNSUPPORTED_FORMAT: &str = "manual_paste_unsupported_format";
const BITMAP_FORMAT: u32 = 2;
struct MemoryLock { handle: HGLOBAL, ptr: *mut u8 }
impl MemoryLock {
    unsafe fn new(handle: HGLOBAL) -> Result<Self, &'static str> {
        let ptr = GlobalLock(handle).cast::<u8>();
        if ptr.is_null() { return Err(BACKUP_FAILED); }
        Ok(Self { handle, ptr })
    }
}
impl Drop for MemoryLock { fn drop(&mut self) { unsafe { let _ = GlobalUnlock(self.handle); } } }
struct Global(HGLOBAL);
impl Global {
    // REAL_GLOBAL_ALLOCATE
}
impl Drop for Global { fn drop(&mut self) { unsafe { let _ = GlobalFree(Some(self.0)); } } }
struct Bitmap(HANDLE);
impl Drop for Bitmap { fn drop(&mut self) { unsafe { let _ = DeleteObject(HGDIOBJ(self.0.0)); } } }
enum SavedData { Global(Global), Bitmap(Bitmap) }
struct Saved { format: u32, data: Option<SavedData>, source: HANDLE }
struct NativeReadLock;
impl Drop for NativeReadLock { fn drop(&mut self) { unsafe { let _ = CloseClipboard(); } } }
fn main() -> std::process::ExitCode {
    if unsafe { OpenClipboard(None) }.is_err() {
        println!("status=failed format_count=0 stage=open code=clipboard_busy");
        return std::process::ExitCode::FAILURE;
    }
    let lock = NativeReadLock;
    let formats = match enumerate() {
        Ok(formats) => formats.len(),
        Err(code) => {
            drop(lock);
            println!("status=failed format_count=0 stage=enumerate code={}", code);
            return std::process::ExitCode::FAILURE;
        }
    };
    let result = snapshot();
    let status = result.as_ref().map(|_| ()).map_err(|&code| code);
    drop(result); // All copied clipboard data is freed in this process, not serialized.
    let closed = unsafe { CloseClipboard() }.is_ok();
    if closed { std::mem::forget(lock); } else { drop(lock); }
    if !closed {
        println!("status=failed format_count={} stage=close code=clipboard_close_failed", formats);
        return std::process::ExitCode::FAILURE;
    }
    match status {
        Ok(()) => {
            println!("status=ok format_count={} stage=complete code=ok", formats);
            std::process::ExitCode::SUCCESS
        }
        Err(code) => {
            println!("status=failed format_count={} stage=snapshot code={}", formats, code);
            std::process::ExitCode::FAILURE
        }
    }
}
'''


def native_snapshot(args: argparse.Namespace, source: str, out: Path) -> int:
    if os.name != "nt":
        raise SystemExit("Native snapshot probe requires Windows")
    if not args.windows_rlib:
        raise SystemExit("Pass --windows-rlib for an already-built windows 0.61.3 rlib")
    rlib = args.windows_rlib.resolve(strict=True)
    functions = ["enumerate", "safe_global_format", "snapshot"]
    if not args.baseline:
        functions.insert(2, "global_matches_snapshot")
        if "fn snapshot_format(" in source:
            functions.insert(3, "snapshot_format")
    bodies = {name: extract_function(source, name) for name in functions}
    policy = (ROOT / "desktop/src-tauri/src/protected_paste_policy.rs").as_posix()
    allocation = extract_function(source, "allocate")
    native_source = NATIVE_ADAPTERS.replace("// REAL_GLOBAL_ALLOCATE", allocation) + f'\n#[path = "{policy}"] mod policy;\n' + "\n".join(bodies.values())
    if not args.baseline:
        native_source = native_source.replace("enum SavedData { Global(Global), Bitmap(Bitmap) }", "enum SavedData { Global(Global, usize), Bitmap(Bitmap) }")
    for forbidden in ["EmptyClipboard", "SetClipboardData", "SendInput", "keybd_event", "CreateWindowExW", "RegisterClassW"]:
        assert forbidden not in native_source, f"Write/input API must never enter read-only probe: {forbidden}"
    label = "baseline" if args.baseline else "current"
    rustfile = out / f"native_snapshot_{label}.rs"
    rustfile.write_text(native_source, encoding="utf-8")
    (out / f"native_manifest_{label}.json").write_text(json.dumps({
        "explicit_native_read_only": True, "clipboard_mutation_api_imports": False,
        "body_output_or_persistence": False,
        "source_ref": "v0.1.6" if args.baseline else "working tree",
        "functions": {name: hashlib.sha256(body.encode()).hexdigest() for name, body in {**bodies, "Global::allocate": allocation}.items()},
    }, indent=2), encoding="utf-8")
    executable = out / f"native_snapshot_{label}.exe"
    rustc = shutil.which("rustc") or str(Path.home() / ".cargo/bin/rustc.exe")
    subprocess.run([rustc, "--edition=2021", "--crate-name", "snapshot_readonly_probe",
                    str(rustfile), "--extern", f"windows={rlib}", "-L", f"dependency={rlib.parent}",
                    "-o", str(executable)], check=True)
    if args.build_native_only:
        print(f"Native snapshot probe compiled only; no clipboard APIs executed. Executable: {executable}")
        return 0
    # Intentionally no output file capture: copied native data stays in process
    # memory; the executable prints only static codes and the format count.
    return subprocess.run([str(executable)], check=False, timeout=45).returncode


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", action="store_true")
    parser.add_argument("--native-snapshot", action="store_true")
    parser.add_argument("--build-native-only", action="store_true")
    parser.add_argument("--windows-rlib", type=Path)
    args = parser.parse_args()
    filename = "desktop/src-tauri/src/protected_paste_windows.rs"
    source = (subprocess.check_output(["git", "show", f"v0.1.6:{filename}"], cwd=ROOT, text=True, encoding="utf-8")
              if args.baseline else (ROOT / filename).read_text(encoding="utf-8"))
    out = ROOT / "outputs/win32-snapshot-017-tests"
    out.mkdir(parents=True, exist_ok=True)
    if args.native_snapshot:
        return native_snapshot(args, source, out)
    if args.build_native_only:
        parser.error("--build-native-only requires --native-snapshot")
    names = ["enumerate", "snapshot"]
    if not args.baseline:
        names.insert(1, "global_matches_snapshot")
        if "fn snapshot_format(" in source:
            names.insert(2, "snapshot_format")
    selected = {name: extract_function(source, name) for name in names}
    policy = (ROOT / "desktop/src-tauri/src/protected_paste_policy.rs").as_posix()
    allocation = extract_function(source, "allocate")
    harness = ADAPTERS.replace("// REAL_GLOBAL_ALLOCATE", allocation) + f'\n#[path = "{policy}"] mod policy;\n' + "\n".join(selected.values()) + TESTS
    if not args.baseline:
        harness = harness.replace("enum SavedData { Global(Global), Bitmap(Bitmap) }", "enum SavedData { Global(Global, usize), Bitmap(Bitmap) }")
    label = "baseline" if args.baseline else "current"
    rustfile = out / f"snapshot_{label}.rs"
    rustfile.write_text(harness, encoding="utf-8")
    (out / f"manifest_{label}.json").write_text(json.dumps({
        "native_clipboard_api_calls": False,
        "source_ref": "v0.1.6" if args.baseline else "working tree",
        "functions": {name: hashlib.sha256(body.encode()).hexdigest() for name, body in {**selected, "Global::allocate": allocation}.items()},
    }, indent=2), encoding="utf-8")
    rustc = shutil.which("rustc") or str(Path.home() / ".cargo/bin/rustc.exe")
    executable = out / (f"snapshot_{label}.exe" if os.name == "nt" else f"snapshot_{label}")
    result = subprocess.run([rustc, "--edition=2021", "--test", str(rustfile), "-o", str(executable)], check=False)
    if result.returncode:
        return result.returncode
    result = subprocess.run([str(executable), "tests::", "--skip", "policy::tests", "--nocapture"], capture_output=True, text=True, timeout=30)
    print(result.stdout, end="")
    print(result.stderr, end="")
    (out / f"{label}-result.txt").write_text(result.stdout + result.stderr, encoding="utf-8")
    return result.returncode


if __name__ == "__main__":
    raise SystemExit(main())
