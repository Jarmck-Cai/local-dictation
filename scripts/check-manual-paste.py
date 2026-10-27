from pathlib import Path
import subprocess
import sys

root = Path(__file__).resolve().parents[1]
source = (root / "desktop/src-tauri/src/transcription_coordinator.rs").read_text(encoding="utf-8")
# Exercise the exact production state machine and its real tests, without Tauri/model linking.
start = source.index("const DEBOUNCE")
end = source.index("/// Serialises all transcription")
commands_start = source.index("/// Commands processed sequentially")
commands_end = source.index("/// Decide whether a key-up")
core = source[start:commands_start] + source[commands_end:end]
tests = source[source.index("#[cfg(test)]\nmod tests"):]
header = """#![allow(dead_code, unused_macros)]
macro_rules! debug { ($($args:tt)*) => { () } }
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
#[derive(Clone, Copy, PartialEq, Eq)]
enum ShortcutActivation { PushToTalk, Toggle, HoldOrToggle }
"""
out = root / "outputs/manual-paste-015-tests"
out.mkdir(exist_ok=True)
(root / "outputs/manual-paste-015-tests/coordinator.rs").write_text(header + core + tests, encoding="utf-8")
rustc = str(Path.home() / ".cargo/bin/rustc.exe")
paths = [out / "coordinator.rs", root / "desktop/src-tauri/src/manual_paste_policy.rs", root / "desktop/src-tauri/src/manual_paste_keyboard.rs", root / "desktop/src-tauri/src/shortcut/press_latch.rs", root / "desktop/src-tauri/src/shortcut/binding_conflicts.rs", root / "desktop/src-tauri/src/shortcut/binding_defaults.rs", root / "desktop/src-tauri/src/shortcut/registration_lifecycle.rs"]
failed = False
for path in paths:
    executable = out / (path.stem + ".exe")
    result = subprocess.run([rustc, "--edition=2021", "--test", str(path), "-o", str(executable)], check=False)
    if result.returncode:
        failed = True
        continue
    result = subprocess.run([str(executable)], check=False)
    failed |= bool(result.returncode)

integration = out / "keyboard_transaction_integration.rs"
policy_path = (root / "desktop/src-tauri/src/protected_paste_policy.rs").as_posix()
keyboard_path = (root / "desktop/src-tauri/src/manual_paste_keyboard.rs").as_posix()
integration.write_text(f"""#![allow(dead_code)]
#[path = \"{policy_path}\"] mod protected;
#[path = \"{keyboard_path}\"] mod keyboard;
""" + r"""
#[cfg(test)]
mod integration_tests {
    use super::*;
    use std::cell::Cell;
    use std::time::Duration;
    struct Provider<'a> { physical: &'a Cell<bool>, restored: bool, published: bool }
    impl protected::Provider for Provider<'_> {
        fn prepare(&mut self, _: &str) -> Result<(), &'static str> { self.physical.set(true); Ok(()) }
        fn publish(&mut self) -> Result<(), &'static str> { self.published = true; Ok(()) }
        fn begin_injection(&mut self) -> Result<(), &'static str> { Ok(()) }
        fn observe(&mut self) -> Result<protected::Observation, &'static str> { panic!("rejected injection never waits for read") }
        fn elapsed(&self) -> Duration { Duration::ZERO }
        fn pause(&mut self) { panic!("rejected injection never pauses") }
        fn restore(&mut self) -> Result<(), &'static str> { self.restored = true; Ok(()) }
    }
    struct Keyboard<'a>(&'a Cell<usize>);
    impl keyboard::Keyboard for Keyboard<'_> {
        fn key(&mut self, _: keyboard::Key, _: keyboard::Direction) -> Result<(), ()> { self.0.set(self.0.get()+1); Ok(()) }
    }
    #[test]
    fn integration_key_pressed_during_backup_cancels_input_and_restores_clipboard() {
        let physical = Cell::new(false);
        let commands = Cell::new(0);
        let mut provider = Provider { physical: &physical, restored: false, published: false };
        assert!(!physical.get()); // original physical release observation
        let result = protected::transact(&mut provider, "synthetic", || {
            keyboard::inject_if_released(&mut Keyboard(&commands), keyboard::Chord::CtrlV,
                || Ok(physical.get()), || false, || {}).map_err(String::from)
        }, || false);
        assert_eq!(result, Err("keys_held".into()));
        assert_eq!(commands.get(), 0);
        assert!(provider.published && provider.restored);
    }
    #[test]
    fn integration_keyboard_unavailable_after_backup_cancels_input_and_restores() {
        let physical = Cell::new(false);
        let commands = Cell::new(0);
        let mut provider = Provider { physical: &physical, restored: false, published: false };
        let result = protected::transact(&mut provider, "synthetic", || {
            keyboard::inject_if_released(&mut Keyboard(&commands), keyboard::Chord::CtrlV,
                || Err("keyboard_unavailable"), || false, || {}).map_err(String::from)
        }, || false);
        assert_eq!(result, Err("keyboard_unavailable".into()));
        assert_eq!(commands.get(), 0);
        assert!(provider.restored);
    }
}
""", encoding="utf-8")
executable = out / "keyboard_transaction_integration.exe"
result = subprocess.run([rustc, "--edition=2021", "--test", str(integration), "-o", str(executable)], check=False)
if result.returncode:
    failed = True
else:
    result = subprocess.run([str(executable), "integration_"], check=False)
    failed |= bool(result.returncode)

sys.exit(1 if failed else 0)
