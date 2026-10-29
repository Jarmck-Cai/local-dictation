"""Run meeting runtime/export tests with synthetic sources and cached dependencies.

No Cargo, capture devices, playback, models, cloud, or user recordings are opened.
"""
from pathlib import Path
import argparse
import os
import shutil
import subprocess


def main():
    root = Path(__file__).resolve().parents[1]
    target = Path(os.environ.get("CARGO_TARGET_DIR", "target"))
    if not target.is_absolute():
        target = root / "desktop/src-tauri" / target
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--deps", type=Path, default=target / "debug/deps", help="Read-only Cargo debug dependencies; prepare them with cargo test --no-run first")
    parser.add_argument("--filter", default="")
    parser.add_argument("--links", action="store_true", help="Include native link fixtures (requires link creation permission)")
    parser.add_argument("--partial-write", action="store_true", help="Inject a real 17-byte failed packet write into an isolated store copy")
    parser.add_argument("--output-dir", type=Path, help="Isolated build directory inside this checkout's ignored outputs/")
    args = parser.parse_args()
    args.deps = args.deps.resolve()
    out = (args.output_dir or root / ("outputs/meeting-runtime-partial-check" if args.partial_write else "outputs/meeting-runtime-check")).resolve()
    if not out.is_relative_to((root / "outputs").resolve()):
        parser.error("--output-dir must be inside this checkout's ignored outputs/")
    out.mkdir(parents=True, exist_ok=True)
    module_root = root / "desktop/src-tauri/src/meetings"
    for name in ("types", "contracts"):
        source = (module_root / f"{name}.rs").read_text(encoding="utf-8").replace(", specta::Type", "")
        (out / f"{name}.rs").write_text(source, encoding="utf-8")
    paths = {name: module_root / f"{name}.rs" for name in ("queue", "store", "alignment", "runtime")}
    if args.partial_write:
        source = paths["store"].read_text(encoding="utf-8")
        anchor = '        file.write_all(&header)\n            .and_then(|_| file.write_all(&packet.data))'
        if source.count(anchor) != 1:
            raise SystemExit("Store packet-write injection point changed; review this synthetic fault harness")
        source = source.replace(anchor, '''        if packet.sequence == 1 && packet.source == Source::Microphone
            && self.manifest.title == "synthetic partial write" {
            file.write_all(&header[..17]).map_err(|_| "meeting_io")?;
            return Err("meeting_io");
        }
''' + anchor)
        source = source.replace('#[path = "store_tests.rs"]', f'#[path = "{(module_root / "store_tests.rs").as_posix()}"]')
        paths["store"] = out / "store.rs"
        paths["store"].write_text(source, encoding="utf-8")
    harness = out / "tests.rs"
    harness.write_text('#![allow(dead_code)]\nmod meetings {\n' + ''.join(
        f'#[path = "{(out / (name + ".rs")).as_posix()}"] mod {name};\n' for name in ("types", "contracts")) + ''.join(
        f'#[path = "{paths[name].as_posix()}"] mod {name};\n' for name in ("queue", "store", "alignment", "runtime")) + '''
mod capture {
    use super::{queue::CaptureSender, types::CaptureConfig};
    use std::sync::{Arc, atomic::AtomicBool};
    pub struct CaptureHandle;
    impl CaptureHandle {
        pub fn start_qpc_100ns(&self)->u64 { 0 }
        pub fn stop(self)->Result<u64,String> { Err("synthetic_only".into()) }
    }
    pub fn start_capture(_:CaptureConfig,_:CaptureSender,_:Arc<AtomicBool>)->Result<CaptureHandle,String> { Err("synthetic_only".into()) }
}
}
''', encoding="utf-8")
    executable = out / "meeting-runtime-tests.exe"
    command = [shutil.which("rustc") or str(Path.home() / ".cargo/bin/rustc.exe"), "--edition=2021", "--test", "-O", str(harness), "-L", f"dependency={args.deps}", "-o", str(executable)]
    for name in ("serde", "serde_json", "rubato", "hound"):
        candidates = sorted(args.deps.glob(f"lib{name}-*.rlib"), key=lambda p: p.stat().st_mtime, reverse=True)
        if not candidates:
            raise SystemExit(f"Missing immutable {name} artifacts in {args.deps}")
        command += ["--extern", f"{name}={candidates[0]}"]
    subprocess.run(command, check=True, cwd=root)
    test_filter = args.filter or ("runtime_partial_packet_write" if args.partial_write else "")
    return subprocess.run([str(executable), test_filter, "--test-threads=1", "--nocapture"] + (["--include-ignored"] if args.links or args.partial_write else []), cwd=root).returncode


if __name__ == "__main__":
    raise SystemExit(main())
