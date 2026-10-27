"""Compile bounded local vocabulary and SQLite checks using existing Cargo artifacts."""
from pathlib import Path
import subprocess

root = Path(__file__).resolve().parents[1]
deps = root / "desktop/src-tauri/target/debug/deps"
rusqlite = next(deps.glob("librusqlite-*.rlib"))
fixture = root / "outputs/vocabulary-check-harness.rs"
fixture.write_text(
    f'#[path = "{(root / "desktop/src-tauri/src/vocabulary_core.rs").as_posix()}"] mod core;\n'
    f'#[path = "{(root / "desktop/src-tauri/src/vocabulary_store.rs").as_posix()}"] mod store;\n',
    encoding="utf-8",
)
executable = root / "outputs/vocabulary-checks.exe"
subprocess.run(["rustc", "--edition=2021", "--test", str(fixture), "--extern", f"rusqlite={rusqlite}", "-L", f"dependency={deps}", "-o", str(executable)], check=True)
subprocess.run([str(executable)], check=True)
