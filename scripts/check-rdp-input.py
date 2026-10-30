"""Offline Unicode policy tests; no system clipboard, keyboard or UI actions."""
import argparse
import hashlib
import json
import os
import re
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--windows-rlib", type=Path)
    parser.add_argument("--build-mstsc-probe", type=Path, help="Compile a fixed-text production-adapter probe; never execute its valid mode")
    args = parser.parse_args()
    if args.build_mstsc_probe and not args.windows_rlib:
        parser.error("--build-mstsc-probe requires --windows-rlib")
    out = ROOT / "outputs/rdp-unicode-tests"
    out.mkdir(parents=True, exist_ok=True)
    rustc = shutil.which("rustc") or str(Path.home() / ".cargo/bin/rustc.exe")
    source = ROOT / "desktop/src-tauri/src/rdp_input.rs"
    # The policy is portable; strip only the Windows-only module declaration so
    # --test cannot link or execute actual input adapters on this Windows host.
    policy = source.read_text(encoding="utf-8")
    policy = policy.replace('#[cfg(target_os = "windows")]\n#[path = "rdp_input_windows.rs"]\npub(crate) mod platform;\n', '')
    generated = out / "policy.rs"
    generated.write_text('#![allow(dead_code)]\n' + policy, encoding="utf-8")
    executable = out / ("policy.exe" if os.name == "nt" else "policy")
    subprocess.run([rustc, "--edition=2021", "--test", str(generated), "-o", str(executable)], check=True)
    result = subprocess.run([str(executable)], check=False, timeout=30).returncode
    if result or not args.windows_rlib:
        return result
    dependency = args.windows_rlib.resolve()
    flags = ["--edition=2021", "--extern", f"windows={dependency}", "-L", f"dependency={dependency.parent}"]
    source_dir = source.parent
    metadata = out / "native_check.rs"
    metadata.write_text('#![allow(dead_code)]\n' + '\n'.join(
        f'#[path = "{(source_dir / name).as_posix()}"] mod {name[:-3]};'
        for name in ["rdp_input.rs", "protected_paste.rs"]), encoding="utf-8")
    subprocess.run([rustc, *flags, "--crate-type=lib", "--emit=metadata", str(metadata), "-o", str(out / "native.rmeta")], check=True)
    native = (source_dir / "rdp_input_windows.rs").read_text(encoding="utf-8")
    # Replace only external Win32 call boundaries. Event construction, process
    # identity/focus code, resource ownership and Runtime implementation stay real.
    native = re.sub(r'\bSendInput,\s*', '', native, count=1)
    native = re.sub(r'use windows::Win32::UI::WindowsAndMessaging::\{[^;]+;\n', '', native, count=1)
    native = re.sub(r'#\[link\(name = "kernel32"\)\]\nextern "system" \{.*?\n\}', '', native, count=1, flags=re.S)
    native_file = out / "native_fake.rs"
    native_file.write_text(native + '\n' + (ROOT / "scripts/rdp-native-cases.rs").read_text(encoding="utf-8"), encoding="utf-8")
    generated_policy = out / "native_policy.rs"
    generated_policy.write_text(policy + f'\n#[path = "{native_file.as_posix()}"] pub(crate) mod platform;\n', encoding="utf-8")
    native_root = out / "native_tests.rs"
    native_root.write_text(f'''#![allow(dead_code)]
#[path = "{generated_policy.as_posix()}"] mod rdp_input;
mod protected_paste {{
    pub(crate) struct CopyLease;
    impl CopyLease {{
        pub(crate) fn prepare(_: &str) -> Result<Self, &'static str> {{ crate::rdp_input::platform::copy_prepare(); Ok(Self) }}
        pub(crate) fn finish(&mut self, cancelled: &dyn Fn()->bool) -> Result<(), &'static str> {{ crate::rdp_input::platform::copy_finish(cancelled); Ok(()) }}
    }}
}}
''', encoding="utf-8")
    native_exe = out / "native_tests.exe"
    subprocess.run([rustc, *flags, "--test", str(native_root), "-o", str(native_exe)], check=True)
    print("Synthetic native API boundaries only; no real desktop input or clipboard calls.", flush=True)
    result = subprocess.run([str(native_exe)], check=False, timeout=30).returncode
    if result or not args.build_mstsc_probe:
        return result
    executable = args.build_mstsc_probe.resolve()
    executable.parent.mkdir(parents=True, exist_ok=True)
    native_probe = executable.with_suffix(".native.rs")
    native_probe.write_text((source_dir / "rdp_input_windows.rs").read_text(encoding="utf-8") + '\n'
                            + (ROOT / "scripts/mstsc-unicode-probe.rs").read_text(encoding="utf-8"), encoding="utf-8")
    policy_probe = executable.with_suffix(".policy.rs")
    policy_probe.write_text(policy + f'\n#[path = "{native_probe.as_posix()}"] pub(crate) mod platform;\n', encoding="utf-8")
    root_probe = executable.with_suffix(".generated.rs")
    root_probe.write_text('#![allow(dead_code)]\n' + '\n'.join([
        f'#[path = "{policy_probe.as_posix()}"] mod rdp_input;',
        *[f'#[path = "{(source_dir / name).as_posix()}"] mod {name[:-3]};' for name in
          ["protected_paste.rs", "keyboard_state_policy.rs", "keyboard_state_windows.rs"]],
        '''fn main() {
    let args: Vec<String> = std::env::args().collect();
    let pid = if args.len() == 3 && args[1] == "--mstsc-unicode-b1" {
        args[2].parse::<u32>().ok().filter(|pid| *pid > 0)
    } else { None };
    let Some(pid) = pid else { std::process::exit(2); };
    if let Err(code) = rdp_input::platform::fixed_probe(pid) {
        eprintln!("{code}"); std::process::exit(1);
    }
}''']), encoding="utf-8")
    subprocess.run([rustc, *flags, "--crate-name=mstsc_unicode_probe", str(root_probe), "-o", str(executable)], check=True)
    files = ["rdp_input.rs", "rdp_input_windows.rs", "protected_paste.rs", "protected_paste_windows.rs",
             "protected_paste_policy.rs", "keyboard_state_policy.rs", "keyboard_state_windows.rs"]
    manifest = {"build_only": True, "valid_mode_executed": False, "fixed_case": "B1",
                "sources": {name: hashlib.sha256((source_dir / name).read_bytes()).hexdigest() for name in files},
                "probe_sha256": hashlib.sha256((ROOT / "scripts/mstsc-unicode-probe.rs").read_bytes()).hexdigest(),
                "generated_sha256": {file.name: hashlib.sha256(file.read_bytes()).hexdigest() for file in [root_probe, policy_probe, native_probe]},
                "binary_sha256": hashlib.sha256(executable.read_bytes()).hexdigest()}
    executable.with_suffix(".manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    print("Built fixed-case B1 probe only; no valid input mode was executed.", flush=True)
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
