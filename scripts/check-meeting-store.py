"""Compile the production packet store against immutable native dependency artifacts.

Synthetic inputs only. Does not invoke Cargo, devices, audio playback, or models.
"""
from pathlib import Path
import argparse
import shutil
import subprocess

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--deps", type=Path)
    parser.add_argument("--build-soak", action="store_true", help="Also build controlled native-format soak executable")
    parser.add_argument("--links", action="store_true", help="Include native hardlink/junction fixtures; requires link creation permission")
    parser.add_argument("--output-dir", type=Path, help="Separate build output when another soak executable is running")
    parser.add_argument("--check-stalls", action="store_true", help="Run controlled 1.5s stall success and 11s stall overflow checks")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    out = args.output_dir or root / "outputs/meeting-store-check"
    if not out.is_absolute(): out = root / out
    out.mkdir(parents=True, exist_ok=True)
    deps = args.deps or root / "desktop/src-tauri/target/debug/deps"
    libs = {}
    for name in ("serde", "serde_json"):
        candidates = sorted(deps.glob(f"lib{name}-*.rlib"), key=lambda p: p.stat().st_mtime, reverse=True)
        if not candidates:
            raise SystemExit(f"Missing immutable {name} build artifacts in {deps}")
        libs[name] = candidates[0]
    types = (root / "desktop/src-tauri/src/meetings/types.rs").read_text(encoding="utf-8").replace(", specta::Type", "")
    (out / "types.rs").write_text(types, encoding="utf-8")
    source = root / "desktop/src-tauri/src/meetings/store.rs"
    queue_source = root / "desktop/src-tauri/src/meetings/queue.rs"
    harness = out / "tests.rs"
    harness.write_text(f'#![allow(dead_code)]\nmod types;\n#[path = "{queue_source.as_posix()}"] mod queue;\n#[path = "{source.as_posix()}"] mod store;\n', encoding="utf-8")
    executable = out / "meeting-store-tests.exe"
    command = [shutil.which("rustc") or str(Path.home()/".cargo/bin/rustc.exe"), "--edition=2021", "--test", str(harness), "-L", f"dependency={deps}", "-o", str(executable)]
    for name, path in libs.items(): command += ["--extern", f"{name}={path}"]
    subprocess.run(command, check=True)
    run_command = [str(executable), "--test-threads=1"]
    if args.links: run_command.append("--include-ignored")
    result = subprocess.run(run_command).returncode
    if result or not (args.build_soak or args.check_stalls):
        return result
    harness.write_text(f'#![allow(dead_code)]\nmod types;\n#[path = "{queue_source.as_posix()}"] mod queue;\n#[path = "{source.as_posix()}"] mod store;\ninclude!("{(root / "scripts/meeting-store-harness.rs").as_posix()}");\n', encoding="utf-8")
    command.remove("--test")
    command.append("-O")
    command[command.index("-o") + 1] = str(out / "meeting-store-soak.exe")
    subprocess.run(command, check=True)
    soak = out / "meeting-store-soak.exe"
    print(soak)
    if args.check_stalls:
        import json
        import time
        stamp = str(time.time_ns())
        fixture = out / "controlled-stalls"
        for milliseconds, seconds, expected in ((1500, 6, 0), (11000, 13, 1)):
            meeting_id = f"stall-{milliseconds}-{stamp}"
            process = subprocess.run([str(soak), "--root", str(fixture), "--id", meeting_id, "--seconds", str(seconds), "--stall-checkpoint-ms", str(milliseconds)], text=True, capture_output=True)
            print(process.stdout, end="")
            print(process.stderr, end="")
            assert process.returncode == expected, f"Controlled {milliseconds}ms stall: unexpected exit"
            manifest = json.loads((fixture / meeting_id / "manifest.json").read_text(encoding="utf-8"))
            if expected:
                assert manifest["status"] == "interrupted" and manifest["error"] == "harness_queue_full"
                diagnostics = [json.loads(line) for line in process.stdout.splitlines() if line.startswith("{")]
                assert any(row.get("queue_peak") == 2048 for row in diagnostics)
            else:
                assert sum(chunk["frames"] for chunk in manifest["chunks"]) == 576000
        print("Controlled checkpoint stalls: PASS (short stall retained all packets; over-budget stall explicitly interrupted)")
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
