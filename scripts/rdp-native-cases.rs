// Synthetic Win32 API boundaries for the real native adapter. No desktop APIs.
use std::cell::RefCell;
#[derive(Default)]
struct System {
    image: String,
    system: String,
    hwnd: usize,
    pid: u32,
    denied: bool,
    query_failed: bool,
    opens: usize,
    closes: usize,
    inputs: Vec<(u16, u32, u16)>,
    calls: usize,
    accept: Option<u32>,
    copy_prepares: usize,
    copy_finishes: usize,
}
thread_local! { static SYSTEM: RefCell<System> = RefCell::new(System::default()); }
fn reset() {
    SYSTEM.with(|s| {
        *s.borrow_mut() = System {
            image: "C:\\Windows\\System32\\mstsc.exe".into(),
            system: "C:\\Windows\\System32".into(),
            hwnd: 11,
            pid: 22,
            ..Default::default()
        }
    });
}
#[allow(non_snake_case)]
unsafe fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void {
    assert_eq!((access, inherit, pid), (0x1000, 0, 22));
    SYSTEM.with(|s| {
        let mut s = s.borrow_mut();
        if s.denied {
            std::ptr::null_mut()
        } else {
            s.opens += 1;
            123usize as *mut _
        }
    })
}
#[allow(non_snake_case)]
unsafe fn QueryFullProcessImageNameW(
    _: *mut std::ffi::c_void,
    flags: u32,
    path: *mut u16,
    units: *mut u32,
) -> i32 {
    assert_eq!(flags, 0);
    SYSTEM.with(|s| {
        let s = s.borrow();
        if s.query_failed {
            return 0;
        }
        let text: Vec<_> = s.image.encode_utf16().collect();
        assert!(text.len() < *units as usize);
        std::ptr::copy_nonoverlapping(text.as_ptr(), path, text.len());
        *units = text.len() as u32;
        1
    })
}
#[allow(non_snake_case)]
unsafe fn GetSystemDirectoryW(path: *mut u16, capacity: u32) -> u32 {
    SYSTEM.with(|s| {
        let text: Vec<_> = s.borrow().system.encode_utf16().collect();
        assert!(text.len() < capacity as usize);
        std::ptr::copy_nonoverlapping(text.as_ptr(), path, text.len());
        text.len() as u32
    })
}
#[allow(non_snake_case)]
unsafe fn CloseHandle(handle: *mut std::ffi::c_void) -> i32 {
    assert!(!handle.is_null(), "never close an invalid process handle");
    SYSTEM.with(|s| s.borrow_mut().closes += 1);
    1
}
#[allow(non_snake_case)]
unsafe fn GetForegroundWindow() -> HWND {
    SYSTEM.with(|s| HWND(s.borrow().hwnd as *mut _))
}
#[allow(non_snake_case)]
unsafe fn GetWindowThreadProcessId(hwnd: HWND, pid: Option<*mut u32>) -> u32 {
    SYSTEM.with(|s| {
        let s = s.borrow();
        if hwnd.0 as usize != s.hwnd {
            return 0;
        }
        if let Some(pid) = pid {
            *pid = s.pid;
        }
        33
    })
}
#[allow(non_snake_case)]
unsafe fn SendInput(inputs: &[INPUT], size: i32) -> u32 {
    assert_eq!(size, size_of::<INPUT>() as i32);
    SYSTEM.with(|s| {
        let mut s = s.borrow_mut();
        s.calls += 1;
        for input in inputs {
            assert_eq!(input.r#type, INPUT_KEYBOARD);
            let k = input.Anonymous.ki;
            s.inputs.push((k.wScan, k.dwFlags.0, k.wVk.0));
        }
        s.accept.unwrap_or(inputs.len() as u32)
    })
}
pub(crate) fn copy_prepare() {
    SYSTEM.with(|s| s.borrow_mut().copy_prepares += 1);
}
pub(crate) fn copy_finish(cancelled: &dyn Fn() -> bool) {
    if !cancelled() {
        SYSTEM.with(|s| s.borrow_mut().copy_finishes += 1);
    }
}

#[cfg(test)]
mod native_tests {
    use super::*;
    #[test]
    fn verified_target_emits_matching_utf16_unicode_pairs_in_one_native_call() {
        reset();
        let target = capture().unwrap().unwrap();
        assert_eq!(
            submit(target, "中😀", false, || false, || Ok(()), |_| Ok(())),
            Ok(())
        );
        SYSTEM.with(|s| {
            let s = s.borrow();
            assert_eq!(
                s.inputs,
                [
                    (0x4e2d, 4, 0),
                    (0x4e2d, 6, 0),
                    (0xd83d, 4, 0),
                    (0xd83d, 6, 0),
                    (0xde00, 4, 0),
                    (0xde00, 6, 0)
                ]
            );
            assert_eq!(
                (s.calls, s.copy_prepares, s.copy_finishes, s.opens, s.closes),
                (1, 0, 0, 1, 1)
            );
        });
    }
    #[test]
    fn extension_ideograph_reaches_os_boundary_as_valid_high_low_pair() {
        reset();
        let target = capture().unwrap().unwrap();
        assert_eq!(
            submit(target, "𠀀", false, || false, || Ok(()), |_| Ok(())),
            Ok(())
        );
        // U+20000 is D840 DC00, not two DC00 units. This checks the actual
        // production INPUT construction, not remote rendering or copy return.
        SYSTEM.with(|s| {
            let s = s.borrow();
            assert_eq!(
                s.inputs,
                [
                    (0xd840, 4, 0),
                    (0xd840, 6, 0),
                    (0xdc00, 4, 0),
                    (0xdc00, 6, 0)
                ]
            );
            assert_eq!((s.calls, s.copy_prepares, s.copy_finishes), (1, 0, 0));
        });
    }
    #[test]
    fn arbitrary_same_named_executable_and_other_clients_are_not_unicode_targets() {
        for image in [
            "C:\\Temp\\mstsc.exe",
            "C:\\Windows\\System32\\msrdc.exe",
            "C:\\Windows\\System32\\notepad.exe",
        ] {
            reset();
            SYSTEM.with(|s| s.borrow_mut().image = image.into());
            assert!(capture().unwrap().is_none());
            SYSTEM.with(|s| {
                let s = s.borrow();
                assert_eq!((s.calls, s.opens, s.closes), (0, 1, 1));
            });
        }
        reset();
        SYSTEM.with(|s| s.borrow_mut().image = "c:\\WINDOWS\\system32\\MSTSC.EXE".into());
        assert!(capture().unwrap().is_some());
    }
    #[test]
    fn missing_foreground_or_process_identity_refuses_input() {
        for mode in 0..3 {
            reset();
            SYSTEM.with(|s| {
                let mut s = s.borrow_mut();
                match mode {
                    0 => s.hwnd = 0,
                    1 => s.denied = true,
                    _ => s.query_failed = true,
                }
            });
            assert!(capture().is_err());
            SYSTEM.with(|s| {
                let s = s.borrow();
                assert_eq!(s.calls, 0);
                assert_eq!(s.opens, s.closes);
            });
        }
    }
    #[test]
    fn focus_or_process_change_during_preparation_prevents_native_submission() {
        for pid_change in [false, true] {
            reset();
            let target = capture().unwrap().unwrap();
            assert_eq!(
                submit(
                    target,
                    "中",
                    false,
                    || false,
                    || {
                        SYSTEM.with(|s| {
                            let mut s = s.borrow_mut();
                            if pid_change {
                                s.pid = 23;
                            } else {
                                s.hwnd = 12;
                            }
                        });
                        Ok(())
                    },
                    |_| Ok(())
                ),
                Err("input_target_changed".into())
            );
            SYSTEM.with(|s| assert_eq!(s.borrow().calls, 0));
        }
    }
    #[test]
    fn keyboard_token_rejection_prevents_native_submission() {
        for code in ["keys_held", "keyboard_unavailable"] {
            reset();
            let target = capture().unwrap().unwrap();
            assert_eq!(
                submit(target, "中", false, || false, || Ok(()), |_| Err(code)),
                Err(code.into())
            );
            SYSTEM.with(|s| assert_eq!(s.borrow().calls, 0));
        }
    }
    #[test]
    fn zero_and_partial_native_return_do_not_retry_or_copy() {
        for (accept, code) in [
            (0, "rdp_input_rejected"),
            (1, "rdp_input_uncertain"),
            (3, "rdp_input_uncertain"),
        ] {
            reset();
            SYSTEM.with(|s| s.borrow_mut().accept = Some(accept));
            let target = capture().unwrap().unwrap();
            assert_eq!(
                submit(target, "中A", true, || false, || Ok(()), |_| Ok(())),
                Err(code.into())
            );
            SYSTEM.with(|s| {
                let s = s.borrow();
                assert_eq!((s.calls, s.copy_prepares, s.copy_finishes), (1, 1, 0));
            });
        }
    }
    #[test]
    fn copy_preference_applies_only_after_all_native_events_are_accepted() {
        reset();
        let target = capture().unwrap().unwrap();
        assert_eq!(
            submit(target, "中A", true, || false, || Ok(()), |_| Ok(())),
            Ok(())
        );
        SYSTEM.with(|s| {
            let s = s.borrow();
            assert_eq!((s.calls, s.copy_prepares, s.copy_finishes), (1, 1, 1));
        });
    }
}
