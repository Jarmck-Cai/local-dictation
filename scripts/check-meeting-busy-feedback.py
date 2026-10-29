"""Run meeting rejection and coordinator regressions without Cargo or devices."""
from pathlib import Path
import shutil
import subprocess
root = Path(__file__).resolve().parents[1]
source = root / "desktop/src-tauri/src"
out = root / "outputs/meeting-busy-feedback"
out.mkdir(parents=True, exist_ok=True)
rustc = shutil.which("rustc") or str(Path.home() / ".cargo/bin/rustc.exe")
coordinator = (source / "transcription_coordinator.rs").read_text(encoding="utf-8")
core_start = coordinator.index("const DEBOUNCE")
commands_start = coordinator.index("/// Commands processed sequentially")
commands_end = coordinator.index("/// Decide whether a key-up")
core_end = coordinator.index("/// Serialises all transcription")
bridge_start = coordinator.index("fn retain_session_capture(")
tests_start = coordinator.index("#[cfg(test)]\nmod tests {")
coordinator_harness = out / "coordinator.rs"
coordinator_harness.write_text('''#![allow(dead_code, unused_macros)]
use std::sync::{Arc, Mutex, Weak, atomic::{AtomicBool, AtomicU64, Ordering}};
use std::time::{Duration, Instant};
macro_rules! debug { ($($args:tt)*) => { () } }
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ShortcutActivation { Toggle, PushToTalk, HoldOrToggle }
''' + f'#[path = "{(source / "recording_gate.rs").as_posix()}"] mod recording_gate;\n'
    + 'use recording_gate::{RecordingGate, RecordingPermit};\n'
    + coordinator[core_start:commands_start] + coordinator[commands_end:core_end]
    + coordinator[coordinator.index("struct CaptureOwnership {"):coordinator.index("#[derive(Debug, Clone, Copy)]\npub enum ManualPasteRejection")]
    + coordinator[bridge_start:tests_start] + coordinator[tests_start:], encoding="utf-8")
coordinator_exe = out / "coordinator.exe"
subprocess.run([rustc, "--edition=2021", "--test", str(coordinator_harness), "-o", str(coordinator_exe)], check=True)
subprocess.run([str(coordinator_exe)], check=True)

notice_exe = out / "overlay-notice.exe"
subprocess.run([rustc, "--edition=2021", "--test", str(source / "overlay_notice.rs"),
                "-o", str(notice_exe)], check=True)
subprocess.run([str(notice_exe)], check=True)
