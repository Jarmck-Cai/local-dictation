//! Classic system MSTSC only. Never redirects focus or uses a paste chord.
use super::{Runtime, UnicodeEvent};
use std::mem::size_of;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
    VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

#[link(name = "kernel32")]
extern "system" {
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
    fn QueryFullProcessImageNameW(
        process: *mut std::ffi::c_void,
        flags: u32,
        path: *mut u16,
        units: *mut u32,
    ) -> i32;
    fn GetSystemDirectoryW(path: *mut u16, size: u32) -> u32;
    fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
}

struct Process(*mut std::ffi::c_void);
impl Drop for Process {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

pub(crate) struct Target {
    hwnd: HWND,
    pid: u32,
    // Keep the queried process object alive until the submission is finished.
    _process: Process,
}
impl Target {
    fn current(&self) -> bool {
        if unsafe { GetForegroundWindow() } != self.hwnd {
            return false;
        }
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(self.hwnd, Some(&mut pid)) != 0 && pid == self.pid }
    }
}

pub(crate) fn capture() -> Result<Option<Target>, String> {
    const UNAVAILABLE: &str = "input_target_unavailable";
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_invalid() {
        return Err(UNAVAILABLE.into());
    }
    let mut pid = 0;
    if unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) } == 0 || pid == 0 {
        return Err(UNAVAILABLE.into());
    }
    let process = Process(unsafe { OpenProcess(0x1000, 0, pid) });
    if process.0.is_null() {
        return Err(UNAVAILABLE.into());
    }
    let mut path = vec![0u16; 32768];
    let mut units = path.len() as u32;
    if unsafe { QueryFullProcessImageNameW(process.0, 0, path.as_mut_ptr(), &mut units) } == 0
        || units == 0
        || units as usize >= path.len()
    {
        return Err(UNAVAILABLE.into());
    }
    let actual = String::from_utf16(&path[..units as usize]).map_err(|_| UNAVAILABLE)?;
    let mut system = [0u16; 32768];
    let count = unsafe { GetSystemDirectoryW(system.as_mut_ptr(), system.len() as u32) } as usize;
    if count == 0 || count >= system.len() {
        return Err(UNAVAILABLE.into());
    }
    let system = String::from_utf16(&system[..count]).map_err(|_| UNAVAILABLE)?;
    let target = Target {
        hwnd,
        pid,
        _process: process,
    };
    if !target.current() {
        return Err("input_target_changed".into());
    }
    if super::is_system_mstsc(&actual, &system) {
        Ok(Some(target))
    } else {
        Ok(None)
    }
}

fn native_events(events: &[UnicodeEvent]) -> Result<Vec<INPUT>, &'static str> {
    let mut result = Vec::new();
    result
        .try_reserve_exact(events.len())
        .map_err(|_| "rdp_input_unavailable")?;
    for event in events {
        result.push(INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0),
                    wScan: event.unit,
                    dwFlags: if event.key_up {
                        KEYEVENTF_UNICODE | KEYEVENTF_KEYUP
                    } else {
                        KEYEVENTF_UNICODE
                    },
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        });
    }
    Ok(result)
}

struct Adapter<P, K, C> {
    target: Target,
    inputs: Vec<INPUT>,
    prepared: Option<P>,
    prepare_keyboard: Option<K>,
    check_keyboard: C,
    copy_result: bool,
    copy: Option<crate::protected_paste::CopyLease>,
}
impl<P, K, C> Runtime for Adapter<P, K, C>
where
    K: FnOnce() -> Result<P, &'static str>,
    C: Fn(&P) -> Result<(), &'static str>,
{
    fn prepare(&mut self, text: &str, events: &[UnicodeEvent]) -> Result<(), &'static str> {
        self.inputs = native_events(events)?;
        if self.copy_result {
            self.copy = Some(crate::protected_paste::CopyLease::prepare(text)?);
        }
        self.prepared = Some(self
            .prepare_keyboard
            .take()
            .ok_or("keyboard_unavailable")?()?);
        Ok(())
    }
    fn ready(&self) -> Result<(), &'static str> {
        (self.check_keyboard)(self.prepared.as_ref().ok_or("keyboard_unavailable")?)?;
        if self.target.current() {
            Ok(())
        } else {
            Err("input_target_changed")
        }
    }
    fn submit(&mut self) -> usize {
        // No allocation, clipboard operation, sleep or key release here.
        unsafe { SendInput(&self.inputs, size_of::<INPUT>() as i32) as usize }
    }
    fn finish_copy(&mut self, cancelled: &dyn Fn() -> bool) -> Result<(), &'static str> {
        if let Some(copy) = self.copy.as_mut() {
            copy.finish(cancelled)?;
        }
        Ok(())
    }
}

pub(crate) fn submit<P>(
    target: Target,
    text: &str,
    copy_result: bool,
    cancelled: impl Fn() -> bool,
    prepare_keyboard: impl FnOnce() -> Result<P, &'static str>,
    check_keyboard: impl Fn(&P) -> Result<(), &'static str>,
) -> Result<(), String> {
    let mut runtime = Adapter {
        target,
        inputs: Vec::new(),
        prepared: None,
        prepare_keyboard: Some(prepare_keyboard),
        check_keyboard,
        copy_result,
        copy: None,
    };
    super::run(&mut runtime, text, cancelled).map_err(String::from)
}
