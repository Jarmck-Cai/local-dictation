"""Check real timing JSON types without loading Tauri, models, or user data.

Requires a previously built native debug dependency cache for serde/serde_json.
Only synthetic JSON and generated test binaries are read/written.
"""
from pathlib import Path
import shutil
import subprocess


def main() -> int:
    root = Path(__file__).resolve().parents[1]
    source = (root / "desktop/src-tauri/src/dictation_metrics.rs").read_text(encoding="utf-8")
    start = source.index("#[derive(Clone, Serialize, Deserialize, specta::Type)]")
    end = source.index("pub fn now_ms()", start)
    types = source[start:end].replace(", specta::Type", "")
    out = root / "outputs/dictation-metadata-016-tests"
    out.mkdir(parents=True, exist_ok=True)
    harness = out / "metadata.rs"
    harness.write_text("use serde::{Deserialize, Serialize};\n" + types + r'''
#[cfg(test)]
mod tests {
    use super::*;
    const OLD: &str = r#"{"id":"synthetic","history_id":1,"recorded_at_ms":1,"asr_ms":2,"cleanup_ms":3,"paste_ms":4,"total_ms":9,"cleanup_budget_ms":0,"mode":"off","outcome":"ready"}"#;
    #[test]
    fn old_metadata_has_unknown_recording_duration() {
        let row: DictationTiming = serde_json::from_str(OLD).unwrap();
        assert_eq!(row.recording_ms, None);
        assert!(row.resource_snapshots.is_none());
    }
    #[test]
    fn new_duration_round_trips_without_changing_other_metadata() {
        let mut json: serde_json::Value = serde_json::from_str(OLD).unwrap();
        json["recording_ms"] = serde_json::json!(15_000);
        let row: DictationTiming = serde_json::from_value(json).unwrap();
        assert_eq!(row.recording_ms, Some(15_000));
        let stored = serde_json::to_value(row).unwrap();
        assert_eq!(stored["recording_ms"], 15_000);
        assert_eq!(stored["total_ms"], 9);
        assert_eq!(stored["id"], "synthetic");
    }
    #[test]
    fn explicit_null_is_unknown_not_zero() {
        let mut json: serde_json::Value = serde_json::from_str(OLD).unwrap();
        json["recording_ms"] = serde_json::Value::Null;
        let row: DictationTiming = serde_json::from_value(json).unwrap();
        assert_eq!(row.recording_ms, None);
    }
}
''', encoding="utf-8")
    deps = root / "desktop/src-tauri/target/debug/deps"
    libraries = {}
    for name in ("serde", "serde_json"):
        candidates = sorted(deps.glob(f"lib{name}-*.rlib"), key=lambda p: p.stat().st_mtime, reverse=True)
        if not candidates:
            raise SystemExit(f"Build native debug dependencies before this check ({name} missing).")
        libraries[name] = candidates[0]
    rustc = shutil.which("rustc") or str(Path.home() / ".cargo/bin/rustc.exe")
    executable = out / "metadata.exe"
    command = [rustc, "--edition=2021", "--test", str(harness), "-L", f"dependency={deps}", "-o", str(executable)]
    for name, library in libraries.items():
        command.extend(["--extern", f"{name}={library}"])
    subprocess.run(command, check=True)
    subprocess.run([str(executable)], check=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
