"""Exercise production meeting processing on synthetic files and native APM.

No Cargo, audio devices, playback, real ASR, or personal recordings. Existing
immutable Rust dependency artifacts and a prepared native library are required.
"""
from pathlib import Path
import argparse
import os
import shutil
import subprocess


def cargo_deps(root):
    target = Path(os.environ.get("CARGO_TARGET_DIR", "target"))
    if not target.is_absolute():
        target = root / "desktop/src-tauri" / target
    return target / "debug/deps"


def prepared_wrapper(root, deps):
    candidates = sorted(
        (deps.parent / "build").glob("handy-*/out/*meeting_apm_wrapper.o"),
        key=lambda path: path.stat().st_mtime_ns, reverse=True,
    )
    sources = list((root / "desktop/src-tauri/native/meeting_apm").glob("*"))
    if not candidates or any(
        path.is_file() and path.stat().st_mtime_ns > candidates[0].stat().st_mtime_ns
        for path in sources
    ):
        raise SystemExit(
            "Missing or stale Cargo-built meeting APM wrapper. First run cargo test "
            "--no-run in desktop/src-tauri, or pass an explicitly prepared --wrapper. "
            "This check never runs Cargo or uses private prototype outputs."
        )
    return candidates[0]


def main():
    root = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--deps", type=Path, default=cargo_deps(root), help="Read-only Cargo debug dependencies; prepare them with cargo test --no-run first")
    apm_root = Path(os.environ.get("MEETING_APM_DIR", root / ".cache/native/webrtc-apm-2.1-msvc-x64-md"))
    if not apm_root.is_absolute():
        apm_root = root / "desktop/src-tauri" / apm_root
    parser.add_argument("--native", type=Path, default=apm_root / "lib")
    parser.add_argument("--ort", type=Path, default=root / ".cache/native/onnxruntime-win-x64-1.24.2/lib")
    parser.add_argument("--wrapper", type=Path, help="Explicit prebuilt wrapper; default is the newest fresh Cargo-built object under the dependency target")
    parser.add_argument("--combined-library", action="store_true")
    parser.add_argument("--filter", default="")
    parser.add_argument("--output-dir", type=Path, default=root / "outputs/meeting-processing-check", help="Isolated build directory inside this checkout's ignored outputs/")
    args = parser.parse_args()
    args.deps = args.deps.resolve()
    args.native = args.native.resolve()
    args.ort = args.ort.resolve()
    if args.wrapper:
        args.wrapper = args.wrapper.resolve()
    out = args.output_dir.resolve()
    if not out.is_relative_to((root / "outputs").resolve()):
        parser.error("--output-dir must be inside this checkout's ignored outputs/")
    if args.combined_library and args.wrapper:
        parser.error("--wrapper and --combined-library are mutually exclusive")
    wrapper = None if args.combined_library else (args.wrapper or prepared_wrapper(root, args.deps))
    if wrapper and not wrapper.is_file():
        parser.error("--wrapper must name an existing immutable wrapper object")
    out.mkdir(parents=True, exist_ok=True)
    libs = {}
    for name in ("serde", "serde_json", "rubato", "vad_rs"):
        candidates = sorted(args.deps.glob(f"lib{name}-*.rlib"), key=lambda p: p.stat().st_mtime, reverse=True)
        if not candidates:
            raise SystemExit(f"Missing immutable {name} artifact in {args.deps}")
        libs[name] = candidates[0]
    modules = root / "desktop/src-tauri/src/meetings"
    for name in ("types", "contracts"):
        (out / f"{name}.rs").write_text((modules / f"{name}.rs").read_text(encoding="utf-8").replace(", specta::Type", ""), encoding="utf-8")
    harness = out / "tests.rs"
    harness.write_text("#![allow(dead_code)]\nmod meetings {\n" + "".join(
        f'#[path = "{((out if name in ("types", "contracts") else modules) / (name + ".rs")).as_posix()}"] mod {name};\n'
        for name in ("types", "contracts", "queue", "store", "alignment", "apm", "processing")) + "}\n", encoding="utf-8")
    executable = out / "meeting-processing-tests.exe"
    command = [shutil.which("rustc") or str(Path.home() / ".cargo/bin/rustc.exe"), "--edition=2021", "--test", "-O", str(harness), "-L", f"dependency={args.deps}", "-L", f"native={args.native}", "-o", str(executable)]
    if not args.combined_library:
        command += ["-C", f"link-arg={wrapper}"]
    for name, path in libs.items():
        command += ["--extern", f"{name}={path}"]
    command += ["-L", f"native={args.ort}"]
    subprocess.run(command, check=True, cwd=root)
    # Windows' System32 may contain an older unrelated ONNX Runtime. Native
    # imports resolve beside the executable before System32, unlike PATH.
    shutil.copy2(args.ort / "onnxruntime.dll", out / "onnxruntime.dll")
    env = os.environ.copy()
    env["PATH"] = str(args.ort) + os.pathsep + env.get("PATH", "")
    return subprocess.run([str(executable), args.filter, "--test-threads=1", "--nocapture"], cwd=out, env=env).returncode


if __name__ == "__main__":
    raise SystemExit(main())
