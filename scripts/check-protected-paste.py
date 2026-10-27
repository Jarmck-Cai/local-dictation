"""Test protected-paste control flow with fake clipboard data; never access OS clipboard.

Optional --windows-rlib reuses an already-built windows 0.61.3 dependency for a
metadata-only Win32 type check. This compiles the platform adapter without running it.
"""
from pathlib import Path
import argparse
import os
import shutil
import subprocess
import tempfile


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--windows-rlib", type=Path)
    args = parser.parse_args()
    if args.windows_rlib and os.name != "nt":
        parser.error("--windows-rlib requires a Windows host")
    root = Path(__file__).resolve().parents[1]
    rustc = shutil.which("rustc")
    if not rustc:
        candidate = Path.home() / ".cargo" / "bin" / ("rustc.exe" if os.name == "nt" else "rustc")
        if candidate.exists():
            rustc = str(candidate)
    if not rustc:
        raise SystemExit("Rust compiler unavailable")
    cache = root / ".cache" / "protected-paste-check"
    cache.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="check-", dir=cache) as directory:
        work = Path(directory)
        executable = work / ("policy-tests.exe" if os.name == "nt" else "policy-tests")
        subprocess.run([
            rustc, "--edition=2021", "--test",
            str(root / "desktop/src-tauri/src/protected_paste_policy.rs"),
            "-o", str(executable),
        ], check=True)
        subprocess.run([str(executable)], check=True)
        if args.windows_rlib:
            rlib = args.windows_rlib.resolve(strict=True)
            harness = work / "windows_check.rs"
            module = (root / "desktop/src-tauri/src/protected_paste.rs").as_posix()
            harness.write_text(
                '#![allow(dead_code)]\n'
                f'#[path = "{module}"]\nmod protected_paste;\nfn main() {{}}\n',
                encoding="utf-8",
            )
            subprocess.run([
                rustc, "--edition=2021", "--emit=metadata", "--crate-name",
                "protected_paste_check", str(harness), "--extern", f"windows={rlib}",
                "-L", f"dependency={rlib.parent}", "-o", str(work / "windows_check.rmeta"),
            ], check=True)
            print("Win32 adapter metadata check passed; no clipboard APIs executed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
