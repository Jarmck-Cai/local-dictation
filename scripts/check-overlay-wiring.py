"""Exercise the actual capture entry, overlay event producer and status consumer.

Only native scheduling/windowing and React hooks are replaced. Production match,
progress wrappers, snapshot/event statements and status branches run unchanged.
No Tauri, audio, model, clipboard or network runtime is started.
"""
from __future__ import annotations

import json
from pathlib import Path
import subprocess


ROOT = Path(__file__).resolve().parents[1]
OUTPUTS = ROOT / "outputs"
ACTIONS = (ROOT / "desktop/src-tauri/src/actions.rs").read_text(encoding="utf-8")
OVERLAY = (ROOT / "desktop/src-tauri/src/overlay.rs").read_text(encoding="utf-8")
CONSUMER = (ROOT / "desktop/src/components/settings/DictationStatus.tsx").read_text(encoding="utf-8")


def function(source: str, signature: str) -> str:
    start = source.index(signature)
    opening = source.index("{", start)
    depth = 1
    cursor = opening + 1
    while depth:
        if source[cursor] == "{":
            depth += 1
        elif source[cursor] == "}":
            depth -= 1
        cursor += 1
    return source[start:cursor]


entry_start = ACTIONS.index("        match settings.overlay_style {")
entry_end = ACTIONS.index("        // Everything above runs", entry_start)
entry = ACTIONS[entry_start:entry_end]
sync_start = OVERLAY.index("fn sync_overlay_on_main(")
sync_end = OVERLAY.index("    let Some(state) = state else", sync_start)
sync = OVERLAY[sync_start:sync_end].replace(
    "fn sync_overlay_on_main(app_handle: &AppHandle) {",
    "fn sync_overlay_on_main(app_handle: &AppHandle) -> Rendered {",
) + "    Rendered { state, update }\n}\n"
snapshot_start = OVERLAY.index("struct OverlayUpdate {")
snapshot_end = OVERLAY.index("\n}", snapshot_start) + 2
snapshot = OVERLAY[snapshot_start:snapshot_end]
wrappers = "\n".join(function(OVERLAY, signature) for signature in (
    "fn show_overlay_state(",
    "pub fn emit_recording_ready(",
    "pub fn show_recording_overlay(",
    "pub fn show_streaming_overlay(",
    "pub fn show_transcribing_overlay(",
    "pub fn show_processing_overlay(",
    "pub fn hide_recording_overlay(",
))
notice_path = (ROOT / "desktop/src-tauri/src/overlay_notice.rs").as_posix()
delivery_path = (ROOT / "desktop/src-tauri/src/paste_delivery.rs").as_posix()
harness = '''#![allow(dead_code)]
use std::sync::Mutex;
#[path = "NOTICE_PATH"]
mod overlay_notice;
#[path = "DELIVERY_PATH"]
mod paste_delivery;
use overlay_notice::OverlayNotice;
static OVERLAY_NOTICE: Mutex<OverlayNotice> = Mutex::new(OverlayNotice::new());
#[derive(Clone, Copy, PartialEq, Eq)]
enum OverlayStyle { None, Minimal, Live }
#[derive(Clone, Copy)]
struct Settings { overlay_style: OverlayStyle }
pub struct AppHandle {
    settings: Settings,
    emitted: Mutex<Vec<(&'static str, &'static str)>>,
}
impl AppHandle {
    fn emit(&self, event: &'static str, state: &'static str) -> Result<(), ()> {
        self.emitted.lock().unwrap().push((event, state));
        Ok(())
    }
}
mod settings {
    use super::*;
    pub fn get_settings(app: &AppHandle) -> Settings { app.settings }
}
mod utils { pub use crate::show_streaming_overlay; }
fn queue_overlay_update(_app: &AppHandle) {}
SNAPSHOT
struct Rendered { state: Option<&'static str>, update: OverlayUpdate }
WRAPPERS
SYNC
fn capture_entry(app: &AppHandle, model_supports_streaming: bool) {
    let settings = settings::get_settings(app);
ENTRY
}
fn scenario(name: &str, style: OverlayStyle, supports_streaming: bool,
    phase: Option<&str>, fault: bool, finish: bool, dismiss: bool, make_visible: bool) {
    *OVERLAY_NOTICE.lock().unwrap() = OverlayNotice::new();
    let mut app = AppHandle { settings: Settings { overlay_style: style }, emitted: Mutex::new(vec![]) };
    capture_entry(&app, supports_streaming);
    emit_recording_ready(&app);
    if fault { OVERLAY_NOTICE.lock().unwrap().report(OverlayNotice::PLAYBACK); }
    match phase {
        Some("transcribing") => show_transcribing_overlay(&app),
        Some("processing") => show_processing_overlay(&app),
        _ => (),
    }
    if finish { hide_recording_overlay(&app); }
    if dismiss {
        let mut notice = OVERLAY_NOTICE.lock().unwrap();
        let generation = notice.failure_generation();
        notice.dismiss(generation);
    }
    if make_visible { app.settings.overlay_style = OverlayStyle::Minimal; }
    let rendered = sync_overlay_on_main(&app);
    let emitted = app.emitted.lock().unwrap();
    assert_eq!(emitted.len(), 1);
    assert_eq!(emitted[0].0, "dictation-state");
    println!("CASE\\t{}\\t{}\\t{}\\t{}\\t{}", name, emitted[0].1,
        rendered.state.unwrap_or("hidden"), rendered.update.failures, rendered.update.capture_ready);
}
fn paste_error_scenario(name: &str, error: &str) {
    *OVERLAY_NOTICE.lock().unwrap() = OverlayNotice::new();
    let app = AppHandle { settings: Settings { overlay_style: OverlayStyle::None }, emitted: Mutex::new(vec![]) };
    OVERLAY_NOTICE.lock().unwrap().report_paste_error(paste_delivery::paste_error_code(error));
    let rendered = sync_overlay_on_main(&app);
    assert_eq!(rendered.update.failures, OverlayNotice::PASTE);
    assert!(rendered.update.failure_generation > 0);
    println!("PASTE\\t{}\\t{}\\t{}", name, rendered.update.paste_error_code,
        rendered.state.unwrap_or("hidden"));
}
fn main() {
    scenario("none-recording", OverlayStyle::None, false, None, false, false, false, false);
    scenario("none-recording-fault", OverlayStyle::None, false, None, true, false, false, false);
    scenario("none-recording-dismiss", OverlayStyle::None, false, None, true, false, true, false);
    scenario("none-to-visible", OverlayStyle::None, false, None, false, false, false, true);
    scenario("transcribing-fault", OverlayStyle::Minimal, false, Some("transcribing"), true, false, false, false);
    scenario("processing-fault", OverlayStyle::Minimal, false, Some("processing"), true, false, false, false);
    scenario("completed-fault", OverlayStyle::Minimal, false, Some("processing"), true, true, false, false);
    scenario("streaming", OverlayStyle::Live, true, None, false, false, false, false);
    scenario("live-without-streaming", OverlayStyle::Live, false, None, false, false, false, false);
    paste_error_scenario("held-key", "keys_held");
    paste_error_scenario("state-unavailable", "keyboard_unavailable");
    paste_error_scenario("restore-failed", "manual_paste_restore_failed");
    paste_error_scenario("unconfirmed", "automatic_paste_unconfirmed");
    paste_error_scenario("unknown", "private synthetic detail");
}
'''.replace("NOTICE_PATH", notice_path).replace("DELIVERY_PATH", delivery_path).replace("SNAPSHOT", snapshot).replace("WRAPPERS", wrappers).replace("SYNC", sync).replace("ENTRY", entry)
OUTPUTS.mkdir(exist_ok=True)
source = OUTPUTS / "overlay-wiring-harness.rs"
executable = OUTPUTS / "overlay-wiring-check.exe"
source.write_text(harness, encoding="utf-8")
subprocess.run(["rustc", "--edition=2021", str(source), "-o", str(executable)], check=True)
run = subprocess.run([str(executable)], check=True, text=True, capture_output=True)
cases = []
paste_errors = []
for line in run.stdout.splitlines():
    if line.startswith("CASE\t"):
        _, name, state, native, failures, ready = line.split("\t")
        cases.append({"name": name, "state": state, "native": native, "failures": int(failures), "ready": ready == "true"})
    elif line.startswith("PASTE\t"):
        _, name, code, native = line.split("\t")
        paste_errors.append({"name": name, "code": code, "native": native})

consumer_start = CONSUMER.index('  const recording = state === "recording"')
consumer_end = CONSUMER.index("  const shortcut =", consumer_start)
consumer = CONSUMER[consumer_start:consumer_end]
node_program = '''
const fs = require("node:fs");
const fixture = JSON.parse(fs.readFileSync(0, "utf8"));
const consume = new Function("state", "available", fixture.consumer + "\\nreturn {recording, busy, status};");
process.stdout.write(JSON.stringify(fixture.cases.map((entry) => ({ ...entry, consumed: consume(entry.state, true) }))));
'''
result = subprocess.run(["node", "-e", node_program], input=json.dumps({"consumer": consumer, "cases": cases}),
                        check=True, text=True, capture_output=True)
cases = json.loads(result.stdout)
expected = {
    "none-recording": ("recording", "hidden", 0, True, "recording"),
    "none-recording-fault": ("recording", "error", 2, True, "recording"),
    "none-recording-dismiss": ("recording", "hidden", 0, True, "recording"),
    "none-to-visible": ("recording", "recording", 0, True, "recording"),
    "transcribing-fault": ("transcribing", "error", 2, True, "transcribing"),
    "processing-fault": ("processing", "error", 2, True, "processing"),
    "completed-fault": ("idle", "error", 2, False, "ready"),
    "streaming": ("streaming", "streaming", 0, True, "recording"),
    "live-without-streaming": ("recording", "recording", 0, True, "recording"),
}
assert {entry["name"] for entry in cases} == set(expected), "Harness did not exercise every case"
failures = []
for entry in cases:
    actual = (entry["state"], entry["native"], entry["failures"], entry["ready"], entry["consumed"]["status"])
    if actual != expected[entry["name"]]:
        failures.append(f"FAIL {entry['name']}: actual={actual!r}; expected={expected[entry['name']]!r}")
    else:
        print(f"PASS {entry['name']}: event={entry['state']}, native={entry['native']}, consumer={entry['consumed']['status']}")
if failures:
    raise AssertionError("\n".join(failures))
print(f"All {len(cases)} actual overlay entry/producer/consumer checks passed; native scheduling and React hooks were not started.")

# Transpile the real frontend whitelist; no React/Tauri runtime is loaded. The
# same parsed code drives the existing toast and recovery-card translation keys.
error_program = '''
const fs = require("node:fs");
const path = require("node:path");
const Module = require("node:module");
const fixture = JSON.parse(fs.readFileSync(0, "utf8"));
const ts = require(path.join(fixture.root, "desktop/node_modules/typescript/lib/typescript.js"));
const filename = path.join(fixture.root, "desktop/src/lib/dictationState.ts");
const compiled = ts.transpileModule(fs.readFileSync(filename, "utf8"), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
}).outputText;
const implementation = new Module(filename);
implementation._compile(compiled, filename);
process.stdout.write(JSON.stringify(fixture.cases.map((entry) => {
    const code = implementation.exports.pasteErrorCode({ code: entry.code });
    const translated = {};
    for (const language of ["en", "zh"]) {
        const strings = JSON.parse(fs.readFileSync(path.join(fixture.root,
            "desktop/src/i18n/locales", language, "translation.json"), "utf8"));
        translated[language] = code === "failed"
            ? { title: strings.errors.pasteFailedTitle, description: strings.errors.pasteFailed }
            : strings.dictation.autoPaste[code];
    }
    return { ...entry, frontend: code, translated };
})));
'''
result = subprocess.run(["node", "-e", error_program], input=json.dumps({"root": str(ROOT), "cases": paste_errors}),
                        check=True, text=True, encoding="utf-8", capture_output=True)
paste_errors = json.loads(result.stdout)
expected_errors = {
    "held-key": "keys_held",
    "state-unavailable": "keyboard_unavailable",
    "restore-failed": "clipboard_restore_failed",
    "unconfirmed": "input_unconfirmed",
    "unknown": "failed",
}
assert {entry["name"] for entry in paste_errors} == set(expected_errors)
for entry in paste_errors:
    assert entry["native"] == "error", entry
    assert entry["code"] == entry["frontend"] == expected_errors[entry["name"]], json.dumps(entry)
    for language in ("en", "zh"):
        message = entry["translated"].get(language)
        assert message and message.get("title") and message.get("description"), entry
    print(f"PASS paste-{entry['name']}: native={entry['code']}, frontend={entry['frontend']}, translated=en/zh")
print(f"All {len(paste_errors)} actual paste-code/overlay/frontend whitelist checks passed; no clipboard or keyboard API executed.")
