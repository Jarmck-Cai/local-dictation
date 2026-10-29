//! Windows metadata commit protocol. Partial native failures retain both copies.
use std::{fs, path::Path, time::Duration};

trait Operations {
    fn exists(&self, path: &Path) -> Result<bool, u32>;
    fn replace(&self, source: &Path, target: &Path, backup: &Path) -> Result<(), u32>;
    fn move_new(&self, source: &Path, target: &Path) -> Result<(), u32>;
    fn remove(&self, path: &Path);
    fn wait(&self, attempt: u32);
}

fn commit(ops: &impl Operations, source: &Path, target: &Path) -> Result<(), u32> {
    let backup = source.with_extension("backup");
    for attempt in 0..4 {
        let result = if ops.exists(target)? {
            ops.replace(source, target, &backup)
        } else {
            ops.move_new(source, target)
        };
        match result {
            Ok(()) => {
                ops.remove(&backup);
                return Ok(());
            }
            Err(code) => {
                // 1176/1177 may already have moved the original to backup.
                // Restore only into an absent target; preserve all other copies.
                if matches!(code, 1176 | 1177) {
                    if ops.exists(target) == Ok(false) && ops.exists(&backup) == Ok(true) {
                        let _ = ops.move_new(&backup, target);
                    }
                    return Err(code);
                }
                // Retry only documented pre-commit access/sharing/lock failures.
                if matches!(code, 5 | 32 | 33) && attempt < 3 {
                    ops.wait(attempt);
                } else {
                    return Err(code);
                }
            }
        }
    }
    unreachable!()
}

struct Native;
impl Operations for Native {
    fn exists(&self, path: &Path) -> Result<bool, u32> {
        path.try_exists()
            .map_err(|e| e.raw_os_error().unwrap_or(0) as u32)
    }
    fn replace(&self, source: &Path, target: &Path, backup: &Path) -> Result<(), u32> {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn ReplaceFileW(
                target: *const u16,
                source: *const u16,
                backup: *const u16,
                flags: u32,
                exclude: *mut std::ffi::c_void,
                reserved: *mut std::ffi::c_void,
            ) -> i32;
        }
        let wide = |p: &Path| {
            p.as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<_>>()
        };
        let (source, target, backup) = (wide(source), wide(target), wide(backup));
        let ok = unsafe {
            ReplaceFileW(
                target.as_ptr(),
                source.as_ptr(),
                backup.as_ptr(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if ok != 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(0) as u32)
        }
    }
    fn move_new(&self, source: &Path, target: &Path) -> Result<(), u32> {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn MoveFileExW(source: *const u16, target: *const u16, flags: u32) -> i32;
        }
        let wide = |p: &Path| {
            p.as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<_>>()
        };
        let (source, target) = (wide(source), wide(target));
        let ok = unsafe { MoveFileExW(source.as_ptr(), target.as_ptr(), 8) };
        if ok != 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(0) as u32)
        }
    }
    fn remove(&self, path: &Path) {
        let _ = fs::remove_file(path);
    }
    fn wait(&self, attempt: u32) {
        std::thread::sleep(Duration::from_millis(10 << attempt));
    }
}

pub(super) fn replace(source: &Path, target: &Path) -> Result<(), &'static str> {
    commit(&Native, source, target).map_err(|code| match code {
        2 => "meeting_atomic_replace_win32_2",
        3 => "meeting_atomic_replace_win32_3",
        5 => "meeting_atomic_replace_win32_5",
        32 => "meeting_atomic_replace_win32_32",
        33 => "meeting_atomic_replace_win32_33",
        80 | 183 => "meeting_atomic_replace_exists",
        112 => "meeting_atomic_replace_win32_112",
        1175 => "meeting_atomic_replace_win32_1175",
        1176 => "meeting_atomic_replace_win32_1176",
        1177 => "meeting_atomic_replace_win32_1177",
        _ => "meeting_atomic_replace",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::{Cell, RefCell},
        collections::HashMap,
    };
    struct Fake {
        files: RefCell<HashMap<String, &'static str>>,
        code: u32,
        restore_fails: bool,
        race: bool,
        replaces: Cell<u32>,
        moves: Cell<u32>,
        waits: Cell<u32>,
    }
    impl Fake {
        fn new(code: u32) -> Self {
            Self {
                files: RefCell::new(HashMap::from([
                    ("source".into(), "new"),
                    ("target".into(), "old"),
                ])),
                code,
                restore_fails: false,
                race: false,
                replaces: Cell::new(0),
                moves: Cell::new(0),
                waits: Cell::new(0),
            }
        }
        fn run(&self) -> Result<(), u32> {
            commit(self, Path::new("source"), Path::new("target"))
        }
        fn contents(&self, key: &str) -> Option<&'static str> {
            self.files.borrow().get(key).copied()
        }
    }
    impl Operations for Fake {
        fn exists(&self, p: &Path) -> Result<bool, u32> {
            Ok(self.files.borrow().contains_key(p.to_str().unwrap()))
        }
        fn replace(&self, _: &Path, _: &Path, _: &Path) -> Result<(), u32> {
            self.replaces.set(self.replaces.get() + 1);
            if self.code == 1177 {
                let mut f = self.files.borrow_mut();
                let old = f.remove("target").unwrap();
                f.insert("source.backup".into(), old);
                if self.race {
                    f.insert("target".into(), "competitor");
                }
            }
            if self.code == 0 {
                let mut f = self.files.borrow_mut();
                let new = f.remove("source").unwrap();
                let old = f.insert("target".into(), new).unwrap();
                f.insert("source.backup".into(), old);
                Ok(())
            } else {
                Err(self.code)
            }
        }
        fn move_new(&self, s: &Path, t: &Path) -> Result<(), u32> {
            self.moves.set(self.moves.get() + 1);
            if self.restore_fails {
                return Err(5);
            }
            let mut f = self.files.borrow_mut();
            if self.race || f.contains_key(t.to_str().unwrap()) {
                f.entry("target".into()).or_insert("competitor");
                return Err(183);
            }
            let v = f.remove(s.to_str().unwrap()).ok_or(2u32)?;
            f.insert(t.to_str().unwrap().into(), v);
            Ok(())
        }
        fn remove(&self, p: &Path) {
            self.files.borrow_mut().remove(p.to_str().unwrap());
        }
        fn wait(&self, _: u32) {
            self.waits.set(self.waits.get() + 1);
        }
    }
    #[test]
    fn partial_failures_restore_old_and_preserve_new_without_retry() {
        for code in [1175, 1176, 1177] {
            let f = Fake::new(code);
            assert_eq!(f.run(), Err(code));
            assert_eq!(f.contents("target"), Some("old"));
            assert_eq!(f.contents("source"), Some("new"));
            assert_eq!(f.replaces.get(), 1);
            assert_eq!(f.waits.get(), 0);
        }
    }
    #[test]
    fn restore_failure_or_competing_target_never_deletes_unique_copies() {
        for race in [false, true] {
            let mut f = Fake::new(1177);
            f.restore_fails = !race;
            f.race = race;
            assert_eq!(f.run(), Err(1177));
            assert_eq!(f.contents("source.backup"), Some("old"));
            assert_eq!(f.contents("source"), Some("new"));
            assert_eq!(
                f.contents("target"),
                if race { Some("competitor") } else { None }
            );
        }
    }
    #[test]
    fn first_creation_and_creation_race_never_replace_a_competitor() {
        for race in [false, true] {
            let mut f = Fake::new(0);
            f.files.borrow_mut().remove("target");
            f.race = race;
            assert_eq!(f.run(), if race { Err(183) } else { Ok(()) });
            assert_eq!(f.replaces.get(), 0);
            assert_eq!(
                f.contents("target"),
                Some(if race { "competitor" } else { "new" })
            );
        }
    }
    #[test]
    fn permanent_access_denial_is_bounded_and_preserves_old_file() {
        let f = Fake::new(5);
        assert_eq!(f.run(), Err(5));
        assert_eq!(f.replaces.get(), 4);
        assert_eq!(f.waits.get(), 3);
        assert_eq!(f.contents("target"), Some("old"));
        assert_eq!(f.contents("source"), Some("new"));
    }
}
