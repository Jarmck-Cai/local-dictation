"""Compile the native capture probe and run pure tests without opening devices.

Only explicit --list enumerates devices. Only explicit --capture opens the two
selected endpoints. Dependencies must already exist; this script never runs
Cargo, downloads files, plays audio, or loads speech models.
"""
from pathlib import Path
import argparse
import os
import shutil
import subprocess


def windows_capture_artifacts(deps):
    """Read Cargo dependency manifests to select the required API projection."""
    required = (
        "windows-0.61.3/", "Win32/Media/Audio/mod.rs",
        "Win32/Devices/FunctionDiscovery/mod.rs",
        "Win32/System/Com/StructuredStorage/mod.rs",
        "Win32/System/Performance/mod.rs", "Win32/System/Threading/mod.rs",
    )
    candidates = []
    for artifact in deps.glob("libwindows-*.rlib"):
        manifest = deps / (artifact.stem.removeprefix("lib") + ".d")
        if manifest.is_file():
            text = manifest.read_text(encoding="utf-8", errors="replace").replace("\\", "/")
            if all(fragment in text for fragment in required):
                candidates.append(artifact)
    return sorted(candidates, key=lambda path: path.stat().st_mtime_ns, reverse=True)


def main():
    root = Path(__file__).resolve().parents[1]
    target = Path(os.environ.get("CARGO_TARGET_DIR", "target"))
    if not target.is_absolute():
        target = root / "desktop/src-tauri" / target
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--deps", type=Path, default=target / "debug/deps", help="Read-only Cargo debug dependencies; prepare them with cargo test --no-run first")
    parser.add_argument("--windows-rlib", type=Path, help="Explicit immutable windows 0.61.3 artifact with capture features; default is newest matching cached artifact")
    parser.add_argument("--output-dir", type=Path, default=root / "outputs/meeting-capture-check", help="Isolated build directory inside this checkout's ignored outputs/")
    parser.add_argument("--filter", default="", help="Optional pure Rust test name filter")
    action = parser.add_mutually_exclusive_group()
    action.add_argument("--list", action="store_true", help="Explicitly enumerate local endpoints after the pure checks")
    action.add_argument("--capture", nargs=2, metavar=("MICROPHONE_ID", "RENDER_ID"), help="Explicitly capture these exact local endpoints after the pure checks")
    parser.add_argument("--seconds", type=float, help="Required with --capture, including decimal durations; allowed range 1..30")
    parser.add_argument("--timings", type=Path, help="Optional --capture output: timing metadata only; refuses to overwrite an existing file")
    args = parser.parse_args()
    args.deps = args.deps.resolve()
    if args.capture:
        if args.seconds is None or not 1.0 <= args.seconds <= 30.0:
            parser.error("--capture requires --seconds in 1..30")
    elif args.seconds is not None or args.timings is not None:
        parser.error("--seconds and --timings require explicit --capture")
    out = args.output_dir.resolve()
    if not out.is_relative_to((root / "outputs").resolve()):
        parser.error("--output-dir must be inside this checkout's ignored outputs/")
    libs = {}
    for name in ("serde", "serde_json", "specta", "windows"):
        candidates = sorted(args.deps.glob(f"lib{name}-*.rlib"), key=lambda path: path.stat().st_mtime_ns, reverse=True)
        if name == "windows":
            candidates = [args.windows_rlib.resolve()] if args.windows_rlib else windows_capture_artifacts(args.deps)
        if not candidates or not candidates[0].is_file():
            raise SystemExit(f"Missing immutable {name} artifact in {args.deps}; first run cargo test --no-run in desktop/src-tauri")
        libs[name] = candidates[0]
    out.mkdir(parents=True, exist_ok=True)
    source = root / "scripts/probe-meeting-capture.rs"
    base = [shutil.which("rustc") or str(Path.home() / ".cargo/bin/rustc.exe"), "--edition=2021", "-O", str(source), "-L", f"dependency={args.deps}"]
    for name, path in libs.items():
        base += ["--extern", f"{name}={path}"]
    tests = out / "meeting-capture-tests.exe"
    executable = out / "probe-meeting-capture.exe"
    subprocess.run(base + ["--test", "-o", str(tests)], cwd=root, check=True)
    subprocess.run(base + ["-o", str(executable)], cwd=root, check=True)
    result = subprocess.run([str(tests), args.filter, "--test-threads=1", "--nocapture"], cwd=root)
    if result.returncode:
        return result.returncode
    if args.list:
        return subprocess.run([str(executable), "list"], cwd=root).returncode
    if args.capture:
        command = [str(executable), "capture", *args.capture, str(args.seconds)]
        if args.timings:
            command += ["--timings", str(args.timings.resolve())]
        return subprocess.run(command, cwd=root).returncode
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
