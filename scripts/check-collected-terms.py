"""Run production collected-term storage against synthetic in-memory histories.

Reads prebuilt Rust dependencies only; all generated files stay in this checkout.
Pass --deps to reuse another checkout's artifacts without running Cargo there.
"""
from pathlib import Path
import argparse
import subprocess

root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument("--deps", type=Path, default=root / "desktop/src-tauri/target/debug/deps")
args = parser.parse_args()
out = root / "outputs/collected-terms"
out.mkdir(parents=True, exist_ok=True)
source = root / "desktop/src-tauri/src"
harness = out / "checks.rs"
harness.write_text("\n".join(
    f'#[path = "{(source / filename).as_posix()}"] mod {name};'
    for name, filename in [
        ("core", "vocabulary_core.rs"),
        ("store", "vocabulary_store.rs"),
        ("collected_core", "vocabulary_collected_core.rs"),
        ("collected_store", "vocabulary_collected_store.rs"),
        ("collected_tests", "vocabulary_collected_tests.rs"),
    ]
), encoding="utf-8")
externs = []
for name in ["rusqlite", "rusqlite_migration", "jieba_rs"]:
    artifact = next(args.deps.glob(f"lib{name}-*.rlib"))
    externs.extend(["--extern", f"{name}={artifact}"])
executable = out / "checks.exe"
subprocess.run(["rustc", "--edition=2021", "--test", str(harness), *externs,
                "-L", f"dependency={args.deps}", "-o", str(executable)], check=True)
subprocess.run([str(executable)], check=True)
