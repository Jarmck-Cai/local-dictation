"""Model-free integration witness, not a real ASR or Tauri run.

Compiles the production meeting entry, the complete batch preparation prefix,
the production per-purpose completion policy, and production slot/gate types.
Only the native ASR body and app settings/state plumbing are replaced: ASR is
identity-success, app is idle with Immediate unloading and a downloaded model.
Cold/second-call failures occur in the untouched production preparation prefix.
"""
from pathlib import Path
import argparse
import subprocess

root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument("--deps", type=Path, default=root / "desktop/src-tauri/target/debug/deps")
args = parser.parse_args()
out = root / "outputs/meeting-model-lifecycle"
out.mkdir(parents=True, exist_ok=True)
manager = (root / "desktop/src-tauri/src/managers/transcription.rs").read_text(encoding="utf-8")
commands = (root / "desktop/src-tauri/src/meetings/commands.rs").read_text(encoding="utf-8")
job_guard = commands[commands.index("#[cfg(all(target_os"):commands.index("fn supported()")]

def function(name):
    start = manager.index("fn " + name + "(")
    body = manager.index("{", start)
    depth, end = 1, body + 1
    while depth:
        depth += (manager[end] == "{") - (manager[end] == "}")
        end += 1
    return manager[start:end]

types = manager[manager.index("#[derive(Default)]\nstruct BatchTranscriptionGate"):manager.index("#[derive(Clone)]\npub struct TranscriptionManager")]
prepare = function("transcribe_inner")
prepare = prepare[:prepare.index("        // Get current settings for configuration")]
completion = function("transcribe_inner")
completion = completion[completion.index("        drop(_batch);"):]
body = prepare + "        self.return_engine(engine, &engine_lease);\n        let final_result = String::from(\"synthetic ASR success\");\n" + completion
text = r'''#![allow(dead_code, unused_variables, unused_mut)]
use std::sync::{Arc, Mutex, MutexGuard, Condvar, atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering}};
use anyhow::Result;
macro_rules! debug { ($($args:tt)*) => { () } }
#[derive(Clone, Copy, PartialEq, Eq)] enum TranscriptionPurpose { Dictation, Meeting }
enum LoadedEngine { SenseVoice { marker: () }, Unsupported }
mod managers { pub mod transcription { pub(crate) type TranscriptionManager = crate::TranscriptionManager; } }
''' + types + r'''
struct TranscriptionManager {
    engine: Arc<Mutex<EngineSlot<LoadedEngine>>>,
    batch_gate: BatchTranscriptionGate,
    is_loading: Mutex<bool>,
    loading_condvar: Condvar,
    load_count: AtomicUsize,
    selected_model: Mutex<String>,
    ordinary_pipeline: AtomicBool,
}
impl TranscriptionManager {
    fn new() -> Self { Self { engine: Arc::new(Mutex::new(EngineSlot::default())),
        batch_gate: BatchTranscriptionGate::default(), is_loading: Mutex::new(false),
        loading_condvar: Condvar::new(), load_count: AtomicUsize::new(0),
        selected_model: Mutex::new("sensevoice".into()), ordinary_pipeline: AtomicBool::new(false) } }
    fn touch_activity(&self) {}
    fn borrow_engine(&self) -> Option<(LoadedEngine, EngineLease, EngineLeaseGuard<LoadedEngine>)> {
        let (engine, lease) = self.engine.lock().unwrap().borrow()?;
        let guard = EngineLeaseGuard { slot: self.engine.clone(), token: lease.token };
        Some((engine, lease, guard))
    }
    fn return_engine(&self, engine: LoadedEngine, lease: &EngineLease) {
        let _ = self.engine.lock().unwrap().return_borrow(lease, engine);
    }
    // The native model is represented by a marker; fixture preloads by the
    // same production ownership protocol and never reads model/user files.
    fn prepare_fixture_model(&self) {
        let mut slot = self.engine.lock().unwrap();
        let (generation, _) = slot.begin_load();
        assert!(slot.finish_load(generation, "sensevoice", LoadedEngine::SenseVoice { marker: () }).is_ok());
        self.load_count.fetch_add(1, Ordering::AcqRel);
    }
    // Only the existing application/native-model loading boundary is replaced.
    // The production preparation prefix decides when this is called, and the
    // real ownership slot decides whether a model is already available.
    fn initiate_model_load(&self) {
        let mut slot = self.engine.lock().unwrap();
        if slot.is_loaded() { return; }
        let (generation, previous) = slot.begin_load();
        assert!(previous.is_none());
        let selected = self.selected_model.lock().unwrap().clone();
        let model = if selected == "sensevoice" { LoadedEngine::SenseVoice { marker: () } }
            else { LoadedEngine::Unsupported };
        assert!(slot.finish_load(generation, &selected, model).is_ok());
        self.load_count.fetch_add(1, Ordering::AcqRel);
    }
    // App fixture: Immediate, no ordinary dictation, no microphone or pending
    // batch. This preserves the production arbitration/unload behavior.
    fn maybe_unload_immediately_for(&self, _: &str, protect_processing: bool) {
        let mut slot = self.engine.lock().unwrap();
        let generation = slot.generation;
        let _ = slot.conditional_unload(generation, || self.batch_gate.has_work()
            || (protect_processing && self.ordinary_pipeline.load(Ordering::Acquire)));
    }
''' + function("transcribe_meeting") + "\n" + function("finish_meeting_model_use") + "\n" + function("maybe_unload_after_transcription") + "\n" + body + r'''
}
''' + job_guard + r'''
#[cfg(test)] mod independent_lifecycle {
use super::*;
#[test] fn cold_meeting_should_prepare_its_downloaded_model() {
    let manager = TranscriptionManager::new();
    let result = manager.transcribe_meeting(vec![0.1; 160]);
    assert!(result.is_ok(), "meeting cold preparation failed: {:?}", result);
}
#[test] fn immediate_meeting_should_transcribe_more_than_one_utterance() {
    let manager = TranscriptionManager::new();
    manager.prepare_fixture_model();
    assert!(manager.transcribe_meeting(vec![0.1; 160]).is_ok(), "fixture first utterance must work");
    let second = manager.transcribe_meeting(vec![0.1; 160]);
    assert!(second.is_ok(), "second utterance lost its model: {:?}", second);
}
#[test] fn warm_meeting_chunks_share_one_load_and_job_finally_unloads() {
    let manager = Arc::new(TranscriptionManager::new());
    {
        let _job = MeetingModelUse(manager.clone());
        assert!(manager.transcribe_meeting(vec![0.1; 160]).is_ok());
        assert!(manager.transcribe_meeting(vec![0.1; 160]).is_ok());
        assert_eq!(manager.load_count.load(Ordering::Acquire), 1, "Immediate applies to the whole meeting job");
        assert!(manager.engine.lock().unwrap().is_loaded());
    }
    assert!(!manager.engine.lock().unwrap().is_loaded());
}
#[test] fn real_job_finally_cleans_up_error_and_cancel_results() {
    for outcome in ["recognition failed", "cancelled"] {
        let manager = Arc::new(TranscriptionManager::new());
        let result = (|| -> Result<()> {
            let _job = MeetingModelUse(manager.clone());
            manager.transcribe_meeting(vec![0.1; 160])?;
            Err(anyhow::anyhow!(outcome))
        })();
        assert!(result.is_err());
        assert!(!manager.engine.lock().unwrap().is_loaded());
    }
}
#[test] fn empty_meeting_chunk_keeps_the_job_model_warm() {
    let manager = TranscriptionManager::new();
    assert!(manager.transcribe_meeting(vec![0.1; 160]).is_ok());
    assert_eq!(manager.transcribe_meeting(vec![]).unwrap(), "");
    assert!(manager.engine.lock().unwrap().is_loaded());
    assert_eq!(manager.load_count.load(Ordering::Acquire), 1);
    manager.finish_meeting_model_use();
    assert!(!manager.engine.lock().unwrap().is_loaded());
}
#[test] fn whole_job_finally_preserves_another_ordinary_processing_pipeline() {
    let manager = Arc::new(TranscriptionManager::new());
    {
        let _job = MeetingModelUse(manager.clone());
        manager.transcribe_meeting(vec![0.1; 160]).unwrap();
        manager.ordinary_pipeline.store(true, Ordering::Release);
    }
    assert!(manager.engine.lock().unwrap().is_loaded());
    manager.ordinary_pipeline.store(false, Ordering::Release);
    manager.finish_meeting_model_use();
    assert!(!manager.engine.lock().unwrap().is_loaded());
}
#[test] fn explicit_unload_between_chunks_is_prepared_again() {
    let manager = TranscriptionManager::new();
    assert!(manager.transcribe_meeting(vec![0.1; 160]).is_ok());
    manager.engine.lock().unwrap().invalidate();
    assert!(manager.transcribe_meeting(vec![0.1; 160]).is_ok());
    assert_eq!(manager.load_count.load(Ordering::Acquire), 2);
}
#[test] fn meeting_preparation_respects_an_explicit_unsupported_selection() {
    let manager = TranscriptionManager::new();
    *manager.selected_model.lock().unwrap() = "explicit unsupported model".into();
    let error = manager.transcribe_meeting(vec![0.1; 160]).unwrap_err();
    assert_eq!(error.to_string(), "meeting_model_unsupported");
    assert_eq!(manager.engine.lock().unwrap().model_id.as_deref(), Some("explicit unsupported model"));
    manager.finish_meeting_model_use();
    assert!(!manager.engine.lock().unwrap().is_loaded());
}
}
'''
harness = out / "meeting-lifecycle.rs"
harness.write_text(text, encoding="utf-8")
deps = args.deps
anyhow = max(deps.glob("libanyhow-*.rlib"), key=lambda p: p.stat().st_mtime)
exe = out / "meeting-lifecycle.exe"
subprocess.run([str(Path.home() / ".cargo/bin/rustc.exe"), "--edition=2021", "--test", str(harness), "--extern", f"anyhow={anyhow}", "-L", f"dependency={deps}", "-o", str(exe)], check=True)
with (out / "meeting-lifecycle.log").open("w", encoding="utf-8") as log:
    result = subprocess.run([str(exe), "--nocapture"], stdout=log, stderr=subprocess.STDOUT)
print((out / "meeting-lifecycle.log").read_text(encoding="utf-8"))
raise SystemExit(result.returncode)


