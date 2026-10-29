// Diagnostic only; appended inside the production Win32 clipboard module.
// Exactly one Ctrl+V to the user-confirmed, foreground Windows MSTSC window.
use super::{keyboard, keyboard_state_windows};
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

pub(super) fn automatic_probe(pid: u32, retain: bool) -> Result<(), String> {
    use enigo::Keyboard;
    verify_mstsc(pid)?;
    keyboard_state_windows::start()?;
    println!("countdown_seconds=5 target_must_remain_foreground=true one_ctrl_v=true");
    thread::sleep(Duration::from_secs(5));
    verify_mstsc(pid)?;
    let target = Target::capture(pid)?;
    let mut enigo =
        enigo::Enigo::new(&enigo::Settings::default()).map_err(|_| "probe_input_unavailable")?;
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
            keyboard::Keyboard::key(&mut self.adapter, key, direction)
        }
    }
    let outcome = policy::transact_with_preflight(
        &mut clipboard,
        PROBE_TEXT,
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
    println!("v_press_attempts={} focus_lost={} local_read_observed={} retain_requested={} remote_insertion_requires_user_confirmation=true",
        presses.get(), focus_lost.get(), outcome == Ok(true), retain);
    if presses.get() > 1 {
        return Err("probe_repeated_input_violation".into());
    }
    if clipboard.ownership_lost() {
        return Err("probe_new_copy_preserved".into());
    }
    outcome.map(|_| ())
}
