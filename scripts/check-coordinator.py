"""Run the production pure coordinator state machine without opening Tauri/audio."""
from pathlib import Path
import subprocess

root = Path(__file__).resolve().parents[1]
source = (root / "desktop/src-tauri/src/transcription_coordinator.rs").read_text(encoding="utf-8")
machine = source[source.index("const DEBOUNCE:"):source.index("/// Serialises all transcription")]
tests = source[source.index("#[cfg(test)]\nmod tests {"):]
prefix = '''#![allow(dead_code, unused_variables)]
use std::time::{Duration, Instant};
macro_rules! debug { ($($arg:tt)*) => { () }; }
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ShortcutActivation { Toggle, PushToTalk, HoldOrToggle }
'''
work = root / ".cache/coordinator-check"
work.mkdir(parents=True, exist_ok=True)
rs = work / "coordinator.rs"
rs.write_text(prefix + machine + tests, encoding="utf-8")
exe = work / "coordinator-tests.exe"
subprocess.run(["rustc", "--edition=2021", "--test", str(rs), "-o", str(exe)], check=True)
raise SystemExit(subprocess.run([str(exe)]).returncode)
