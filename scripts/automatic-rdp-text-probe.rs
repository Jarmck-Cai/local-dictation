// Diagnostic only; appended inside the production Win32 clipboard module.
// One explicitly selected Ctrl+V or fixed Unicode batch to a confirmed MSTSC.
use super::{keyboard, keyboard_state_windows};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyboardLayout, MapVirtualKeyExW, MAPVK_VK_TO_VSC_EX,
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
    fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
}

fn verify_mstsc(pid: u32) -> Result<(), &'static str> {
    if pid == 0 {
        return Err("probe_invalid_pid");
    }
    let process = unsafe { OpenProcess(0x1000, 0, pid) }; // QUERY_LIMITED_INFORMATION
    if process.is_null() {
        return Err("probe_target_unavailable");
    }
    let mut path = [0u16; 32768];
    let mut units = path.len() as u32;
    let read = unsafe { QueryFullProcessImageNameW(process, 0, path.as_mut_ptr(), &mut units) };
    let closed = unsafe { CloseHandle(process) };
    if read == 0 || closed == 0 || units == 0 || units as usize >= path.len() {
        return Err("probe_target_identity");
    }
    let actual =
        String::from_utf16(&path[..units as usize]).map_err(|_| "probe_target_identity")?;
    let expected = std::env::var_os("SystemRoot").ok_or("probe_system_root")?;
    let expected = std::path::PathBuf::from(expected)
        .join("System32")
        .join("mstsc.exe");
    if !actual.eq_ignore_ascii_case(&expected.to_string_lossy()) {
        return Err("probe_not_system_mstsc");
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct Target {
    hwnd: HWND,
    pid: u32,
}
impl Target {
    fn capture(pid: u32) -> Result<Self, &'static str> {
        let hwnd = unsafe { GetForegroundWindow() };
        let target = Self { hwnd, pid };
        if !target.current() {
            return Err("probe_target_not_foreground");
        }
        Ok(target)
    }
    fn current(&self) -> bool {
        if self.hwnd.is_invalid() || unsafe { GetForegroundWindow() } != self.hwnd {
            return false;
        }
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(self.hwnd, Some(&mut pid)) != 0 && pid == self.pid }
    }
}

// Only preparation differs: never snapshot unknown original clipboard text.
struct FixedClipboard(WinClipboard);
impl Provider for FixedClipboard {
    fn prepare(&mut self, text: &str) -> Result<(), &'static str> {
        self.0.prepare_fixed(text)
    }
    fn publish(&mut self) -> Result<(), &'static str> {
        self.0.publish()
    }
    fn begin_injection(&mut self) -> Result<(), &'static str> {
        self.0.begin_injection()
    }
    fn observe(&mut self) -> Result<Observation, &'static str> {
        self.0.observe()
    }
    fn elapsed(&self) -> Duration {
        self.0.elapsed()
    }
    fn pause(&mut self) {
        self.0.pause();
    }
    fn restore(&mut self) -> Result<(), &'static str> {
        self.0.restore()
    }
    fn retain_text(&mut self) -> Result<(), &'static str> {
        self.0.retain_text()
    }
    fn ownership_lost(&self) -> bool {
        self.0.ownership_lost()
    }
    fn settle(&mut self, copy: bool, cancelled: &dyn Fn() -> bool) -> Result<(), &'static str> {
        self.0.settle(copy, cancelled)
    }
}

pub(super) fn automatic_probe(
    pid: u32,
    retain: bool,
    scancode: bool,
    unicode: bool,
) -> Result<(), String> {
    use enigo::Keyboard;
    verify_mstsc(pid)?;
    keyboard_state_windows::start()?;
    println!(
        "countdown_seconds=5 target_must_remain_foreground=true one_ctrl_v={} one_unicode_batch={}",
        !unicode, unicode
    );
    thread::sleep(Duration::from_secs(5));
    verify_mstsc(pid)?;
    let target = Target::capture(pid)?;
    // Cache wScan before the gesture; this adapter does not remap on release.
    // Enigo::raw still consults the current layout for VK/extended-key flags.
    // This diagnostic is deliberately limited to nonextended Ctrl and V.
    let raw_codes = if scancode {
        let thread = unsafe { GetWindowThreadProcessId(target.hwnd, None) };
        if thread == 0 {
            return Err("probe_scan_target_unavailable".into());
        }
        let layout = unsafe { GetKeyboardLayout(thread) };
        let mut codes = [0u16; 2];
        for (index, vk) in [0x11, 0x56].iter().enumerate() {
            let scan = unsafe { MapVirtualKeyExW(*vk, MAPVK_VK_TO_VSC_EX, Some(layout)) };
            if scan == 0 || scan > u16::MAX as u32 {
                return Err("probe_scan_mapping_failed".into());
            }
            codes[index] = scan as u16;
        }
        Some(codes)
    } else {
        None
    };
    let mut enigo =
        enigo::Enigo::new(&enigo::Settings::default()).map_err(|_| "probe_input_unavailable")?;
    if unicode {
        // Fixed BMP-only sample. Enigo 0.6.1 Windows text() queues one SendInput
        // batch using KEYEVENTF_UNICODE. No clipboard object is even created.
        let text = "轻述远程输入测试 RDP U1";
        if text.chars().any(|c| c.is_control()) {
            return Err("probe_control_character".into());
        }
        let prepared = keyboard_state_windows::prepare(0)?;
        if !target.current() {
            return Err(policy::CANCELLED.into());
        }
        if keyboard_state_windows::fast_check(&prepared, 0)? {
            return Err("keys_held".into());
        }
        if !target.current() {
            return Err(policy::CANCELLED.into());
        }
        let result = enigo
            .text(text)
            .map_err(|_| "probe_unicode_injection_failed_no_retry".to_owned());
        println!("unicode_batch_calls=1 os_accepted_all={} clipboard_api_calls=0 remote_insertion_requires_user_confirmation=true", result.is_ok());
        return result;
    }
    let mut clipboard = FixedClipboard(WinClipboard::new(retain)?);
    let prepared = RefCell::new(None);
    let presses = Cell::new(0usize);
    let focus_lost = Cell::new(false);
    let cancelled = || {
        let lost = !target.current();
        if lost {
            focus_lost.set(true);
        }
        lost
    };
    // PRODUCTION_ENIGO_ADAPTER
    struct Guarded<'a, 'b> {
        adapter: Adapter<'a>,
        target: Target,
        presses: &'b Cell<usize>,
        focus_lost: &'b Cell<bool>,
        raw_codes: Option<[u16; 2]>,
    }
    impl keyboard::Keyboard for Guarded<'_, '_> {
        fn key(&mut self, key: keyboard::Key, direction: keyboard::Direction) -> Result<(), ()> {
            if direction == keyboard::Direction::Press {
                if !self.target.current() {
                    self.focus_lost.set(true);
                    return Err(());
                }
                if key == keyboard::Key::V {
                    if self.presses.get() != 0 {
                        return Err(());
                    }
                    self.presses.set(1);
                }
            }
            // Releases always settle every attempted key, including partial errors.
            if let Some(codes) = self.raw_codes {
                let scan = match key {
                    keyboard::Key::Control => codes[0],
                    keyboard::Key::V => codes[1],
                    _ => return Err(()),
                };
                let direction = match direction {
                    keyboard::Direction::Press => enigo::Direction::Press,
                    keyboard::Direction::Release => enigo::Direction::Release,
                };
                self.adapter.0.raw(scan, direction).map_err(|_| ())
            } else {
                keyboard::Keyboard::key(&mut self.adapter, key, direction)
            }
        }
    }
    // Per-case labels distinguish this publication from an old identical cache.
    let text = if scancode {
        "轻述远程输入测试 RDP S1"
    } else {
        "轻述远程输入测试 RDP V1"
    };
    let outcome = policy::transact_with_preflight(
        &mut clipboard,
        text,
        || {
            if cancelled() {
                return Err(policy::CANCELLED.into());
            }
            // This probe registers no shortcut bindings. Physical-key and
            // unknown-baseline protections remain the actual production policy.
            *prepared.borrow_mut() = Some(keyboard_state_windows::prepare(0)?);
            Ok(())
        },
        || {
            let prepared = prepared.borrow();
            let prepared = prepared.as_ref().ok_or("keyboard_unavailable")?;
            keyboard::inject_if_released(
                &mut Guarded {
                    adapter: Adapter(&mut enigo),
                    target,
                    presses: &presses,
                    focus_lost: &focus_lost,
                    raw_codes,
                },
                keyboard::Chord::CtrlV,
                || keyboard_state_windows::fast_check(prepared, 0),
                &cancelled,
                || thread::sleep(Duration::from_millis(100)),
            )
            .map_err(String::from)
        },
        &cancelled,
        retain,
    );
    println!("v_press_attempts={} focus_lost={} local_read_observed={} retain_requested={} scancode={} remote_insertion_requires_user_confirmation=true",
        presses.get(), focus_lost.get(), outcome == Ok(true), retain, scancode);
    if presses.get() > 1 {
        return Err("probe_repeated_input_violation".into());
    }
    if clipboard.ownership_lost() {
        return Err("probe_new_copy_preserved".into());
    }
    outcome.map(|_| ())
}
