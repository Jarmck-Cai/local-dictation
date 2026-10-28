"""Run production history search against synthetic SQLite using read-only Rust deps."""
from pathlib import Path
import argparse
import subprocess

root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--deps", type=Path, default=root / "desktop/src-tauri/target/debug/deps")
args = parser.parse_args()
source = (root / "desktop/src-tauri/src/managers/history.rs").read_text(encoding="utf-8")


def block(marker: str) -> str:
    assert marker in source, f"missing production contract: {marker}"
    start = source.index(marker)
    opening = source.index("{", start)
    depth = 1
    end = opening + 1
    while depth:
        depth += (source[end] == "{") - (source[end] == "}")
        end += 1
    return source[start:end]


parts = ["use anyhow::{anyhow, Result};", "use rusqlite::{params, Connection};"]
for name in ("HistoryEntry", "HistorySearchPage"):
    parts.append("#[derive(Debug)]\n" + block(f"pub struct {name} {{"))
parts.append("struct HistoryManager;\nimpl HistoryManager {")
parts += [block("fn map_history_entry("), block("fn search_history_entries_with_conn(")]
parts.append("}")
tests = (root / "desktop/src-tauri/src/managers/history_search_tests.rs").as_posix()
parts.append(f'#[path = "{tests}"] mod tests;')
output = root / "outputs/home-history"
output.mkdir(parents=True, exist_ok=True)
harness = output / "search-harness.rs"
harness.write_text("\n".join(parts), encoding="utf-8")
executable = output / "search-checks.exe"
command = ["rustc", "--edition=2021", "--test", str(harness)]
for name in ("anyhow", "rusqlite"):
    artifact = next(args.deps.glob(f"lib{name}-*.rlib"))
    command += ["--extern", f"{name}={artifact}"]
command += ["-L", f"dependency={args.deps}", "-o", str(executable)]
subprocess.run(command, check=True)
subprocess.run([str(executable)], check=True)
