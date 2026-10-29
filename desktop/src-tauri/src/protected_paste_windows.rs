//! Win32 implementation. All mutation is scoped to one calling worker thread.
//! Backup is best effort; ownership and source-consistency checks remain strict.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::mem::size_of;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{
    GetLastError, GlobalFree, SetLastError, ERROR_SUCCESS, HANDLE, HGLOBAL, HINSTANCE, HWND,
    LPARAM, LRESULT, WPARAM,
};
use windows::Win32::Graphics::Gdi::{DeleteObject, GetObjectW, BITMAP, HGDIOBJ};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData,
    GetClipboardFormatNameW, GetClipboardOwner, GetClipboardSequenceNumber, OpenClipboard,
    RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE, GMEM_ZEROINIT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CopyImage, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetWindowLongPtrW,
    PeekMessageW, RegisterClassW, SetWindowLongPtrW, UnregisterClassW, GWLP_USERDATA, HWND_MESSAGE,
    IMAGE_BITMAP, LR_CREATEDIBSECTION, MSG, PM_REMOVE, WINDOW_EX_STYLE, WINDOW_STYLE,
    WM_DESTROYCLIPBOARD, WM_RENDERALLFORMATS, WM_RENDERFORMAT, WNDCLASSW,
};

use super::policy::{self, Budget, Observation, Provider, MAX_FORMATS, RESTORE_FAILED};

const UNICODE_TEXT: u32 = 13;
const BITMAP_FORMAT: u32 = 2;
const BACKUP_FAILED: &str = "manual_paste_backup_failed";
const UNAVAILABLE: &str = "manual_paste_unavailable";
const BUSY: &str = "manual_paste_clipboard_busy";
const PUBLISH_FAILED: &str = "manual_paste_publish_failed";
const RENDER_FAILED: &str = "manual_paste_render_failed";
const MAX_TEXT_BYTES: usize = 8 * 1024 * 1024;
const OPEN_RESTORE_TIMEOUT: Duration = Duration::from_millis(500);
const POLL: Duration = Duration::from_millis(10);

/// Unlock a successfully locked memory handle on every early-return path.
struct MemoryLock {
    handle: HGLOBAL,
    ptr: *mut u8,
}
impl MemoryLock {
    unsafe fn new(handle: HGLOBAL) -> Result<Self, &'static str> {
        let ptr = GlobalLock(handle).cast::<u8>();
        if ptr.is_null() {
            return Err(BACKUP_FAILED);
        }
        Ok(Self { handle, ptr })
    }
}
impl Drop for MemoryLock {
    fn drop(&mut self) {
        unsafe {
            let _ = GlobalUnlock(self.handle);
        }
    }
}

/// Before transfer we own HGLOBAL; SetClipboardData success transfers it to OS.
struct Global(HGLOBAL);
impl Global {
    fn allocate(bytes: usize) -> Result<Self, &'static str> {
        unsafe {
            // GlobalAlloc may round capacity up. A later clipboard transfer must
            // never expose uninitialized bytes beyond the copied source length.
            GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, bytes)
                .map(Self)
                .map_err(|_| BACKUP_FAILED)
        }
    }
    fn from_bytes(bytes: &[u8]) -> Result<Self, &'static str> {
        let result = Self::allocate(bytes.len())?;
        unsafe {
            let locked = MemoryLock::new(result.0)?;
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), locked.ptr, bytes.len());
        }
        Ok(result)
    }
    fn transfer(self, format: u32) -> Result<(), &'static str> {
        unsafe {
            SetClipboardData(format, Some(HANDLE(self.0 .0))).map_err(|_| PUBLISH_FAILED)?;
        }
        std::mem::forget(self);
        Ok(())
    }
}
impl Drop for Global {
    fn drop(&mut self) {
        unsafe {
            let _ = GlobalFree(Some(self.0));
        }
    }
}

struct Bitmap(HANDLE);
impl Drop for Bitmap {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(HGDIOBJ(self.0 .0));
        }
    }
}

enum SavedData {
    Global(Global, usize),
    Bitmap(Bitmap),
}
struct Saved {
    format: u32,
    data: Option<SavedData>,
    source: HANDLE,
}
impl Saved {
    fn restore(&mut self) -> Result<(), &'static str> {
        // A failed transfer drops its local resource and is reported explicitly.
        // The caller still attempts every remaining format before returning.
        let data = self.data.take().ok_or(RESTORE_FAILED)?;
        match data {
            SavedData::Global(global, _) => {
                global.transfer(self.format).map_err(|_| RESTORE_FAILED)
            }
            SavedData::Bitmap(bitmap) => {
                unsafe {
                    SetClipboardData(self.format, Some(bitmap.0)).map_err(|_| RESTORE_FAILED)?;
                }
                std::mem::forget(bitmap);
                Ok(())
            }
        }
    }
}

struct ClipboardLock;
impl ClipboardLock {
    fn open(hwnd: HWND) -> Result<Self, &'static str> {
        unsafe {
            OpenClipboard(Some(hwnd)).map_err(|_| BUSY)?;
        }
        Ok(Self)
    }
    fn close(self) -> Result<(), &'static str> {
        let result = unsafe { CloseClipboard() }.map_err(|_| UNAVAILABLE);
        // Retain Drop's second close attempt if the explicit close failed.
        if result.is_ok() {
            std::mem::forget(self);
        }
        result
    }
}
impl Drop for ClipboardLock {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

#[derive(Default)]
struct Context {
    text: RefCell<Option<Global>>,
    expected_sequence: Cell<u32>,
    injection_started: Cell<bool>,
    read_at: Cell<Option<Instant>>,
    ownership_lost: Cell<bool>,
    render_failed: Cell<bool>,
    internal_empty: Cell<bool>,
}

unsafe fn context(hwnd: HWND) -> *const Context {
    GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Context
}

/// Called under the requester's existing clipboard lock for WM_RENDERFORMAT.
unsafe fn render(hwnd: HWND, context: &Context, receipt: bool) {
    if GetClipboardOwner().ok() != Some(hwnd) || context.ownership_lost.get() {
        return;
    }
    let Some(text) = context.text.borrow_mut().take() else {
        return;
    };
    if text.transfer(UNICODE_TEXT).is_err() {
        context.render_failed.set(true);
        return;
    }
    // Rendering our promise changes the clipboard sequence. Capture that change
    // while the requesting consumer holds the lock; otherwise restoring after a
    // legitimate read would be mistaken for a newer user's copy.
    let sequence = GetClipboardSequenceNumber();
    if sequence == 0 {
        context.render_failed.set(true);
        return;
    }
    context.expected_sequence.set(sequence);
    if receipt && context.injection_started.get() {
        context.read_at.set(Some(Instant::now()));
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let Some(context) = context(hwnd).as_ref() else {
        return DefWindowProcW(hwnd, message, wp, lp);
    };
    match message {
        WM_RENDERFORMAT if wp.0 as u32 == UNICODE_TEXT => {
            render(hwnd, context, true);
            LRESULT(0)
        }
        WM_RENDERALLFORMATS => {
            // Destruction rendering does not count as a consumer read.
            if let Ok(lock) = ClipboardLock::open(hwnd) {
                render(hwnd, context, false);
                let _ = lock.close();
            }
            LRESULT(0)
        }
        WM_DESTROYCLIPBOARD => {
            if !context.internal_empty.get() {
                context.ownership_lost.set(true);
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, message, wp, lp),
    }
}

struct Window {
    hwnd: HWND,
    instance: HINSTANCE,
    class: Vec<u16>,
}
impl Window {
    fn create(context: &Context) -> Result<Self, &'static str> {
        static NEXT_CLASS: AtomicU64 = AtomicU64::new(0);
        let class: Vec<u16> = format!(
            "LocalDictationProtectedPaste-{}-{}",
            std::process::id(),
            NEXT_CLASS.fetch_add(1, Ordering::Relaxed)
        )
        .encode_utf16()
        .chain(Some(0))
        .collect();
        unsafe {
            let instance = HINSTANCE(GetModuleHandleW(None).map_err(|_| UNAVAILABLE)?.0);
            let definition = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: instance,
                lpszClassName: PCWSTR(class.as_ptr()),
                ..Default::default()
            };
            if RegisterClassW(&definition) == 0 {
                return Err(UNAVAILABLE);
            }
            let hwnd = match CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                PCWSTR(class.as_ptr()),
                w!("LocalDictationProtectedPaste"),
                WINDOW_STYLE::default(),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(instance),
                None,
            ) {
                Ok(hwnd) => hwnd,
                Err(_) => {
                    let _ = UnregisterClassW(PCWSTR(class.as_ptr()), Some(instance));
                    return Err(UNAVAILABLE);
                }
            };
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, context as *const Context as isize);
            Ok(Self {
                hwnd,
                instance,
                class,
            })
        }
    }
    fn pump(&self) {
        unsafe {
            let mut message = MSG::default();
            // Scope dispatch to our window: do not consume a caller's WM_QUIT or
            // unrelated worker messages. Bound one pass even under message spam.
            for _ in 0..64 {
                if !PeekMessageW(&mut message, Some(self.hwnd), 0, 0, PM_REMOVE).as_bool() {
                    break;
                }
                DispatchMessageW(&message);
            }
        }
    }
}
impl Drop for Window {
    fn drop(&mut self) {
        unsafe {
            // Context remains alive until after the window's destruction.
            if DestroyWindow(self.hwnd).is_err() {
                // Even an unexpected OS cleanup failure must not leave the
                // window pointing at Context after its Box is dropped.
                SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
            }
            let _ = UnregisterClassW(PCWSTR(self.class.as_ptr()), Some(self.instance));
        }
    }
}

fn enumerate() -> Result<Vec<u32>, &'static str> {
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    result
        .try_reserve_exact(MAX_FORMATS)
        .map_err(|_| BACKUP_FAILED)?;
    let mut format = 0;
    loop {
        unsafe {
            SetLastError(ERROR_SUCCESS);
            format = EnumClipboardFormats(format);
            if format == 0 {
                if GetLastError() != ERROR_SUCCESS {
                    return Err(BACKUP_FAILED);
                }
                return Ok(result);
            }
        }
        // The 64-format budget limits copied resources, not format metadata.
        // Clipboard IDs fit in 16 bits; reject malformed/cyclic enumeration.
        if format > u16::MAX as u32 {
            return Err(BACKUP_FAILED);
        }
        seen.try_reserve(1).map_err(|_| BACKUP_FAILED)?;
        if !seen.insert(format) {
            return Err(BACKUP_FAILED);
        }
        result.try_reserve(1).map_err(|_| BACKUP_FAILED)?;
        result.push(format);
    }
}

fn safe_global_format(format: u32) -> bool {
    if format < 0xc000 {
        return policy::safe_global_format(format, None);
    }
    let mut buffer = [0u16; 256];
    let count = unsafe { GetClipboardFormatNameW(format, &mut buffer) };
    if count <= 0 {
        return false;
    }
    policy::safe_global_format(
        format,
        Some(&String::from_utf16_lossy(&buffer[..count as usize])),
    )
}

fn global_matches_snapshot(
    saved: &Global,
    bytes: usize,
    current: HGLOBAL,
) -> Result<bool, &'static str> {
    unsafe {
        // Compare complete backed-up data, not system-owned handle identity.
        // Synthesized CF_TEXT/CF_OEMTEXT may return a fresh HGLOBAL each time.
        // Allocation-size differences remain a conservative refusal here.
        // Our GlobalAlloc can round up, so never compare its extra capacity.
        if bytes == 0 || GlobalSize(saved.0) < bytes || GlobalSize(current) != bytes {
            return Ok(false);
        }
        let saved = MemoryLock::new(saved.0)?;
        let current = MemoryLock::new(current)?;
        Ok(std::slice::from_raw_parts(saved.ptr, bytes)
            == std::slice::from_raw_parts(current.ptr, bytes))
    }
}

/// Called only for known storage kinds, while the original clipboard is locked.
fn snapshot_format(format: u32, budget: &mut Budget) -> Result<Saved, &'static str> {
    // A failed allocation/read must not spend the budget for later formats.
    let mut next_budget = *budget;
    let source = unsafe { GetClipboardData(format) }.map_err(|_| BACKUP_FAILED)?;
    let data = if format == BITMAP_FORMAT {
        unsafe {
            let mut bitmap = BITMAP::default();
            if GetObjectW(
                HGDIOBJ(source.0),
                size_of::<BITMAP>() as i32,
                Some((&mut bitmap as *mut BITMAP).cast()),
            ) != size_of::<BITMAP>() as i32
            {
                return Err(BACKUP_FAILED);
            }
            // CopyImage may produce a 32bpp DIB section. Charge the worst
            // case before allocation, plus a bounded header/palette allowance.
            let bytes = policy::bitmap_bytes(
                bitmap.bmWidth,
                bitmap.bmHeight,
                bitmap.bmWidthBytes,
                bitmap.bmPlanes,
                bitmap.bmBitsPixel,
            )?;
            next_budget.add(bytes)?;
            let copy = CopyImage(source, IMAGE_BITMAP, 0, 0, LR_CREATEDIBSECTION)
                .map_err(|_| BACKUP_FAILED)?;
            if copy == source {
                return Err(BACKUP_FAILED);
            }
            SavedData::Bitmap(Bitmap(copy))
        }
    } else {
        unsafe {
            let source_global = HGLOBAL(source.0);
            let bytes = GlobalSize(source_global);
            if bytes == 0 {
                return Err(BACKUP_FAILED);
            }
            next_budget.add(bytes)?;
            let copy = Global::allocate(bytes)?;
            {
                let src = MemoryLock::new(source_global)?;
                let dest = MemoryLock::new(copy.0)?;
                std::ptr::copy_nonoverlapping(src.ptr, dest.ptr, bytes);
            }
            SavedData::Global(copy, bytes)
        }
    };
    *budget = next_budget;
    Ok(Saved {
        format,
        data: Some(data),
        source,
    })
}

fn snapshot() -> Result<Vec<Saved>, &'static str> {
    let formats = enumerate()?;
    let mut result = Vec::new();
    result
        .try_reserve_exact(formats.len().min(MAX_FORMATS))
        .map_err(|_| BACKUP_FAILED)?;
    let mut budget = Budget::default();
    for &format in &formats {
        // Never reinterpret unknown handles as HGLOBAL memory. Unsupported,
        // unreadable and over-budget individual formats are deliberately skipped.
        if format != BITMAP_FORMAT && !safe_global_format(format) {
            continue;
        }
        if let Ok(saved) = snapshot_format(format, &mut budget) {
            result.push(saved);
        }
    }
    let mut verified = Vec::new();
    verified
        .try_reserve_exact(result.len())
        .map_err(|_| BACKUP_FAILED)?;
    for saved in result {
        let Ok(current) = (unsafe { GetClipboardData(saved.format) }) else {
            continue;
        };
        let unchanged = match saved.data.as_ref() {
            Some(SavedData::Global(global, bytes)) => {
                let Ok(unchanged) = global_matches_snapshot(global, *bytes, HGLOBAL(current.0))
                else {
                    continue;
                };
                unchanged
            }
            // Bitmap handling is unchanged; no byte-memory assumptions for GDI handles.
            Some(SavedData::Bitmap(_)) => current == saved.source,
            None => false,
        };
        if !unchanged {
            return Err(BACKUP_FAILED);
        }
        verified.push(saved);
    }
    // A delayed provider can add/change formats while it renders under our lock.
    // Check after the final retrieval too, before any clipboard mutation.
    if enumerate()? != formats {
        return Err(BACKUP_FAILED);
    }
    Ok(verified)
}

fn text_memory(text: &str) -> Result<Global, &'static str> {
    let invalid = "manual_paste_invalid_text";
    if text.is_empty() || text.contains('\0') {
        return Err(invalid);
    }
    let units = text.encode_utf16().count().checked_add(1).ok_or(invalid)?;
    let bytes = units.checked_mul(size_of::<u16>()).ok_or(invalid)?;
    if bytes > MAX_TEXT_BYTES {
        return Err(policy::LIMIT_EXCEEDED);
    }
    let memory = Global::allocate(bytes)?;
    unsafe {
        let locked = MemoryLock::new(memory.0)?;
        let ptr = locked.ptr.cast::<u16>();
        for (index, unit) in text.encode_utf16().chain(Some(0)).enumerate() {
            ptr.add(index).write(unit);
        }
    }
    Ok(memory)
}

struct WinClipboard {
    // Drop the window before the Box it refers to. held is explicitly cleared in Drop.
    window: Window,
    context: Box<Context>,
    held: Option<ClipboardLock>,
    saved: Vec<Saved>,
    restore_snapshot: bool,
    markers: Vec<(u32, Global)>,
    marker_formats: [u32; 3],
    mutated: bool,
    began: Instant,
    retained_text: Option<Global>,
}
impl WinClipboard {
    fn new(_copy_result: bool) -> Result<Self, &'static str> {
        let context = Box::<Context>::default();
        let window = Window::create(&context)?;
        Ok(Self {
            window,
            context,
            held: None,
            saved: Vec::new(),
            restore_snapshot: false,
            markers: Vec::new(),
            marker_formats: [0; 3],
            mutated: false,
            began: Instant::now(),
            retained_text: None,
        })
    }
    fn current_sequence(&self) -> Result<u32, &'static str> {
        let sequence = unsafe { GetClipboardSequenceNumber() };
        if sequence == 0 {
            Err(UNAVAILABLE)
        } else {
            Ok(sequence)
        }
    }
    fn empty(&self) -> Result<(), &'static str> {
        self.context.internal_empty.set(true);
        let result = unsafe { EmptyClipboard() }.map_err(|_| PUBLISH_FAILED);
        self.context.internal_empty.set(false);
        result
    }
}
impl Provider for WinClipboard {
    fn prepare(&mut self, text: &str) -> Result<(), &'static str> {
        *self.context.text.borrow_mut() = Some(text_memory(text)?);
        // Also needed when no original payload can be saved, or a later restore
        // fails. Allocate before EmptyClipboard, never after destroying data.
        self.retained_text = Some(text_memory(text)?);
        // Allocate every restoration and marker resource before the first mutation.
        self.markers
            .try_reserve_exact(3)
            .map_err(|_| BACKUP_FAILED)?;
        for (name, value) in [
            ("ExcludeClipboardContentFromMonitorProcessing", 1u32),
            ("CanIncludeInClipboardHistory", 0u32),
            ("CanUploadToCloudClipboard", 0u32),
        ] {
            let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
            let format = unsafe { RegisterClipboardFormatW(PCWSTR(name.as_ptr())) };
            if format == 0 {
                return Err(UNAVAILABLE);
            }
            self.markers
                .push((format, Global::from_bytes(&value.to_le_bytes())?));
        }
        self.marker_formats = std::array::from_fn(|index| self.markers[index].0);
        self.held = Some(ClipboardLock::open(self.window.hwnd)?);
        self.current_sequence()?;
        let originally_empty = enumerate()?.is_empty();
        self.saved = snapshot()?;
        // Locale and privacy markers alone are not recoverable user content.
        self.restore_snapshot = originally_empty
            || self
                .saved
                .iter()
                .any(|saved| saved.format != 16 && !self.marker_formats.contains(&saved.format));
        // Intentionally keep this same lock through publication: user copies
        // cannot slip between the complete backup and EmptyClipboard.
        Ok(())
    }
    fn publish(&mut self) -> Result<(), &'static str> {
        // Refuse inaccessible sequence state immediately before the first write.
        self.current_sequence()?;
        self.empty()?;
        self.mutated = true;
        self.context.expected_sequence.set(self.current_sequence()?);
        for (format, memory) in self.markers.drain(..) {
            memory.transfer(format)?;
            self.context
                .expected_sequence
                .set(unsafe { GetClipboardSequenceNumber() });
        }
        unsafe {
            SetLastError(ERROR_SUCCESS);
            // A NULL delayed-render handle has NULL success return as well.
            let _ = SetClipboardData(UNICODE_TEXT, None);
            if GetLastError() != ERROR_SUCCESS {
                return Err(PUBLISH_FAILED);
            }
        }
        self.context.expected_sequence.set(self.current_sequence()?);
        self.held.take().ok_or(PUBLISH_FAILED)?.close()?;
        // CloseClipboard can synthesize text formats and advance the sequence.
        // The lease must describe the completed publication, not its open write
        // phase. begin_injection still checks this sequence AND our unique owner
        // under the next clipboard lock, so a newer user's copy is refused.
        self.context.expected_sequence.set(self.current_sequence()?);
        self.began = Instant::now();
        Ok(())
    }
    fn begin_injection(&mut self) -> Result<(), &'static str> {
        self.window.pump();
        let lock = ClipboardLock::open(self.window.hwnd)?;
        let owns = unsafe { GetClipboardOwner() }.ok() == Some(self.window.hwnd);
        let sequence = self.current_sequence()?;
        let valid = policy::same_lease(
            self.context.expected_sequence.get(),
            sequence,
            owns,
            self.context.ownership_lost.get(),
        );
        lock.close()?;
        if !valid {
            return Err("manual_paste_clipboard_changed");
        }
        self.context.injection_started.set(true);
        Ok(())
    }
    fn observe(&mut self) -> Result<Observation, &'static str> {
        self.window.pump();
        if self.context.render_failed.get() {
            return Err(RENDER_FAILED);
        }
        let read_at = self.context.read_at.get();
        Ok(Observation {
            read: read_at.is_some(),
            quiet_for: read_at.map(|at| at.elapsed()),
            ownership_lost: self.context.ownership_lost.get(),
        })
    }
    fn elapsed(&self) -> Duration {
        self.began.elapsed()
    }
    fn pause(&mut self) {
        thread::sleep(POLL);
    }
    fn restore(&mut self) -> Result<(), &'static str> {
        self.settle(false, &|| false)
    }
    fn retain_text(&mut self) -> Result<(), &'static str> {
        self.settle(true, &|| false)
    }
    fn ownership_lost(&self) -> bool {
        self.context.ownership_lost.get()
    }
    fn settle(
        &mut self,
        copy_result: bool,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<(), &'static str> {
        WinClipboard::settle(self, copy_result, is_cancelled)
    }
}
impl WinClipboard {
    fn settle(
        &mut self,
        copy_result: bool,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Result<(), &'static str> {
        if !self.mutated {
            return policy::settlement_result(policy::settlement_cancelled(is_cancelled));
        }
        let started = Instant::now();
        let lock = loop {
            if let Some(lock) = self.held.take() {
                break lock;
            }
            self.window.pump();
            match ClipboardLock::open(self.window.hwnd) {
                Ok(lock) => break lock,
                Err(_) if started.elapsed() < OPEN_RESTORE_TIMEOUT => thread::sleep(POLL),
                Err(_) => return Err(RESTORE_FAILED),
            }
        };
        // Both comparisons MUST be inside this very lock, immediately before
        // EmptyClipboard. A new user's copy always wins over our old snapshot.
        let owns = unsafe { GetClipboardOwner() }.ok() == Some(self.window.hwnd);
        let sequence = self.current_sequence().map_err(|_| RESTORE_FAILED)?;
        let same_lease = policy::same_lease(
            self.context.expected_sequence.get(),
            sequence,
            owns,
            self.context.ownership_lost.get(),
        );
        // Settlement commits at this cancellation sample under the acquired
        // clipboard lock. Esc during OpenClipboard retries restores the snapshot
        // rather than retaining output. A cancellation after this commit point
        // cannot revoke an already committed clipboard mutation.
        let cancelled = policy::settlement_cancelled(is_cancelled);
        let action = policy::settlement_action(copy_result, same_lease, cancelled);
        if action == policy::SettlementAction::PreserveNewCopy {
            self.context.ownership_lost.set(true);
            self.mutated = false;
            lock.close().map_err(|_| RESTORE_FAILED)?;
            return policy::settlement_result(cancelled);
        }
        if action == policy::SettlementAction::RetainResult || !self.restore_snapshot {
            let retained = self.retained_text.take().ok_or(RESTORE_FAILED)?;
            self.empty().map_err(|_| RESTORE_FAILED)?;
            self.context.expected_sequence.set(self.current_sequence()?);
            if retained.transfer(UNICODE_TEXT).is_ok() {
                // The retained result is real clipboard memory, not a promise
                // owned by the temporary window that is about to be destroyed.
                self.context.text.borrow_mut().take();
                self.mutated = false;
                lock.close().map_err(|_| RESTORE_FAILED)?;
                return policy::settlement_result(cancelled);
            }
            // Publication failed after EmptyClipboard. Keep the same lock and
            // restore whatever was saved before reporting a real system failure.
            for saved in &mut self.saved {
                let _ = saved.restore();
            }
            self.mutated = false;
            let _ = lock.close();
            return Err(RESTORE_FAILED);
        }
        self.empty().map_err(|_| RESTORE_FAILED)?;
        let mut restored_payload = false;
        for saved in &mut self.saved {
            if saved.restore().is_ok()
                && saved.format != 16
                && !self.marker_formats.contains(&saved.format)
            {
                restored_payload = true;
            }
        }
        if !self.saved.is_empty() && !restored_payload {
            // Partial original content is useful and remains internally
            // consistent: never mix it with the current transcript. Only when
            // every payload restore failed do we replace it with current text.
            // No second paste chord is sent in either case.
            let retained = self.retained_text.take().ok_or(RESTORE_FAILED)?;
            self.empty().map_err(|_| RESTORE_FAILED)?;
            self.context.expected_sequence.set(self.current_sequence()?);
            retained
                .transfer(UNICODE_TEXT)
                .map_err(|_| RESTORE_FAILED)?;
        }
        self.context.text.borrow_mut().take();
        self.mutated = false;
        lock.close().map_err(|_| RESTORE_FAILED)?;
        policy::settlement_result(cancelled)
    }
}
impl Drop for WinClipboard {
    fn drop(&mut self) {
        self.held.take();
    }
}

/// Optional result copy after direct input; never publishes a temporary body.
pub(crate) struct CopyLease {
    clipboard: WinClipboard,
    sequence: u32,
    owner: Option<HWND>,
}
impl CopyLease {
    pub(crate) fn prepare(text: &str) -> Result<Self, &'static str> {
        let mut clipboard = WinClipboard::new(true)?;
        clipboard.prepare(text)?;
        let sequence = clipboard.current_sequence()?;
        let owner = unsafe { GetClipboardOwner() }.ok();
        // Snapshot and identity refer to the same lock. Never adopt a newer
        // sequence after releasing it; even a close-time update invalidates copy.
        clipboard.held.take().ok_or(UNAVAILABLE)?.close()?;
        Ok(Self {
            clipboard,
            sequence,
            owner,
        })
    }
    pub(crate) fn finish(&mut self, cancelled: &dyn Fn() -> bool) -> Result<(), &'static str> {
        let expected = std::mem::take(&mut self.sequence);
        if expected == 0 {
            return Ok(());
        }
        let started = Instant::now();
        let lock = loop {
            match ClipboardLock::open(self.clipboard.window.hwnd) {
                Ok(lock) => break lock,
                Err(_) if started.elapsed() < OPEN_RESTORE_TIMEOUT => thread::sleep(POLL),
                Err(_) => return Err(RESTORE_FAILED),
            }
        };
        let current = self.clipboard.current_sequence()?;
        let same_owner = unsafe { GetClipboardOwner() }.ok() == self.owner;
        // Check source and cancellation under the very lock that will mutate.
        if current != expected || !same_owner || cancelled() {
            lock.close()?;
            return Ok(());
        }
        let retained = self.clipboard.retained_text.take().ok_or(RESTORE_FAILED)?;
        let mut mutated = false;
        let result = (|| {
            self.clipboard.empty()?;
            mutated = true;
            for (format, memory) in self.clipboard.markers.drain(..) {
                memory.transfer(format)?;
            }
            retained.transfer(UNICODE_TEXT)
        })();
        if result.is_err() && mutated {
            // A copy failure is reported after Unicode submission. Restore what
            // was backed up, without ever sending or retrying keyboard input.
            let _ = self.clipboard.empty();
            for saved in &mut self.clipboard.saved {
                let _ = saved.restore();
            }
        }
        let closed = lock.close();
        if result.is_err() || closed.is_err() {
            Err(RESTORE_FAILED)
        } else {
            Ok(())
        }
    }
}

pub(super) fn run(
    text: &str,
    inject: impl FnOnce() -> Result<(), String>,
    is_cancelled: impl Fn() -> bool,
) -> Result<bool, String> {
    let mut clipboard = WinClipboard::new(false).map_err(String::from)?;
    policy::transact(&mut clipboard, text, inject, is_cancelled)
}

pub(super) fn run_automatic(
    text: &str,
    inject: impl FnOnce() -> Result<(), String>,
    is_cancelled: impl Fn() -> bool,
    copy_result: bool,
) -> Result<bool, String> {
    let mut clipboard = WinClipboard::new(copy_result).map_err(String::from)?;
    policy::transact_with_retention(&mut clipboard, text, inject, is_cancelled, copy_result)
}

pub(super) fn run_with_preflight(
    text: &str,
    preflight: impl FnOnce() -> Result<(), String>,
    inject: impl FnOnce() -> Result<(), String>,
    is_cancelled: impl Fn() -> bool,
    copy_result: bool,
) -> Result<bool, String> {
    let mut clipboard = WinClipboard::new(copy_result).map_err(String::from)?;
    policy::transact_with_preflight(
        &mut clipboard,
        text,
        preflight,
        inject,
        is_cancelled,
        copy_result,
    )
}
