"""Exercise production recording and ASR text policies with synthetic data.

Uses existing Rust dependency artifacts without Cargo, devices, models, user
settings, clipboard access, or cloud services.
"""
from pathlib import Path
import argparse
import os
import shutil
import subprocess

root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument("--deps", type=Path, default=root / "desktop/src-tauri/target/debug/deps")
args = parser.parse_args()
source = root / "desktop/src-tauri/src"
out = root / "outputs/meeting-dictation-bridge"
out.mkdir(parents=True, exist_ok=True)
rustc = shutil.which("rustc") or str(Path.home() / ".cargo/bin/rustc.exe")


def function(text, name):
    start = text.index("fn " + name + "(")
    body = text.index("{", start)
    depth = 1
    end = body + 1
    while depth:
        depth += (text[end] == "{") - (text[end] == "}")
        end += 1
    return text[start:end]


settings = (source / "settings.rs").read_text(encoding="utf-8")
settings = settings[:settings.index("pub fn load_or_create_app_settings")]
settings = settings[:settings.rindex("/// Startup entry point.")]
settings = settings.replace('use tauri::AppHandle;', '').replace('use tauri_plugin_store::StoreExt;', '')
settings = settings.replace('"shortcut/binding_defaults.rs"', f'"{(source / "shortcut/binding_defaults.rs").as_posix()}"')
(out / "settings.rs").write_text(settings, encoding="utf-8")
manager = (source / "managers/transcription.rs").read_text(encoding="utf-8")
purpose_start = manager.index("#[derive(Clone, Copy, PartialEq, Eq)]\nenum TranscriptionPurpose")
purpose_end = manager.index("fn post_process_transcription_text", purpose_start)
test_names = [
    "meeting_error_releases_borrow_and_batch_before_final_model_cleanup",
    "late_batch_guard_cannot_abandon_the_next_borrow_of_the_same_model",
    "late_stream_guard_and_stale_return_cannot_replace_a_new_borrow",
    "unwinding_guard_abandons_only_its_own_borrow",
    "timed_idle_cleanup_rechecks_borrow_activity_and_new_load_atomically",
    "normal_processing_completion_still_unloads_immediately_after_return",
    "engine_failure_guards_do_not_invalidate_a_replacement_generation",
    "failed_arming_cleanup_waits_for_background_load_then_unloads_its_session",
    "conditional_unload_cannot_invalidate_a_new_borrow_after_its_preflight",
    "conditional_unload_cannot_remove_a_new_load_or_arming_session",
    "explicit_model_change_invalidates_old_borrow_even_with_the_same_model_id",
    "loaded_query_and_model_change_do_not_wait_for_a_borrowed_engine",
    "batch_inference_waits_without_making_the_borrowed_model_look_unloaded",
    "batch_failure_and_unwind_release_active_model_borrow",
    "immediate_unload_protects_arming_capture_tails_and_other_batch_work",
    "meeting_preserves_remote_fillers_and_names_despite_personal_preferences",
    "ordinary_dictation_still_applies_personal_preferences",
    "meeting_keeps_explicit_language_and_chinese_script_preferences",
]
harness = out / "policy.rs"
harness.write_text('''#![allow(dead_code, unused_imports)]
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex, MutexGuard, atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering}};
use std::time::Duration;
use std::{sync::mpsc, thread};
use log::{debug, error};
mod settings;
use settings::{AppSettings, ChineseScript};
''' + f'#[path = "{(source / "audio_toolkit/text.rs").as_posix()}"] mod text;\n'
    + f'#[path = "{(source / "chinese_script.rs").as_posix()}"] mod chinese_script;\n'
    + f'#[path = "{(source / "audio_toolkit/lang_id.rs").as_posix()}"] mod lang_id;\n'
    + '''use lang_id::detect_output_language;
use text::{apply_custom_words, normalize_transcription_output,
    remove_filler_words, OutputLanguageEvidence};
use chinese_script::{convert_chinese_script, ChineseVariety};
'''
    + manager[manager.index("#[derive(Default)]\nstruct BatchTranscriptionGate"):manager.index("#[derive(Clone)]\npub struct TranscriptionManager")]
    + manager[purpose_start:purpose_end]
    + function(manager, "post_process_transcription_text")
    + function(manager, "fail_open_text_transform<F>")
    + function(manager, "panic_payload_message")
    + "\n#[cfg(test)] mod bridge_tests { use super::*;\n"
    + 'fn languages(codes: &[&str]) -> Vec<String> { codes.iter().map(|s| s.to_string()).collect() }\n'
    + "\n".join("#[test]\n" + function(manager, name) for name in test_names)
    + "\n}\n", encoding="utf-8")
externs = []
for name in ["serde", "serde_json", "specta", "log", "tauri_plugin_os", "tauri_plugin_log", "ferrous_opencc", "natural", "once_cell", "regex", "strsim", "whatlang", "isolang"]:
    artifacts = sorted(args.deps.glob(f"lib{name}-*.rlib"), key=lambda p: p.stat().st_mtime, reverse=True)
    if not artifacts:
        raise SystemExit(f"Missing prebuilt dependency: {name}; pass --deps after a native build")
    externs += ["--extern", f"{name}={artifacts[0]}"]
exe = out / "policy.exe"
native = []
for directory in (Path.home() / ".cargo/registry/src").glob("*/windows_x86_64_msvc-*/lib"):
    native += ["-L", f"native={directory}"]
subprocess.run([rustc, "--edition=2021", "--test", str(harness), *externs,
                "-L", f"dependency={args.deps}", *native, "-o", str(exe)],
               env={**os.environ, "CARGO_PKG_VERSION": "0.1.12"}, check=True)
subprocess.run([str(exe)], check=True)
gate_exe = out / "recording-gate.exe"
subprocess.run([rustc, "--edition=2021", "--test", str(source / "recording_gate.rs"),
                "-o", str(gate_exe)], check=True)
subprocess.run([str(gate_exe)], check=True)

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
