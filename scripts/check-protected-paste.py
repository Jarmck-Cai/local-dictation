"""Test protected-paste control flow with fake clipboard data; never access OS clipboard.

Optional --windows-rlib reuses an already-built windows 0.61.3 dependency for a
metadata-only Win32 type check. This compiles the platform adapter without running it.
--native-text-memory additionally executes only the production UTF-16 allocation
helper and fixed Windows code-page conversions. It does not create windows, call
clipboard APIs, inject keys, or read any personal text.
"""
from pathlib import Path
import argparse
import hashlib
import json
import os
import shutil
import subprocess
import tempfile


TEXT_MEMORY_PROBE = r'''
#[link(name = "kernel32")]
extern "system" {
    fn WideCharToMultiByte(codepage: u32, flags: u32, wide: *const u16, units: i32,
        out: *mut u8, bytes: i32, default: *const u8, used: *mut i32) -> i32;
}
fn converted(text: &str, codepage: u32) -> (Vec<u8>, bool) {
    let wide: Vec<u16> = text.encode_utf16().collect();
    let mut used = 0;
    let flag = if codepage == 65001 { std::ptr::null_mut() } else { &mut used };
    let size = unsafe { WideCharToMultiByte(codepage, 0, wide.as_ptr(), wide.len() as i32,
        std::ptr::null_mut(), 0, std::ptr::null(), flag) };
    assert!(size > 0);
    let mut bytes = vec![0u8; size as usize];
    let written = unsafe { WideCharToMultiByte(codepage, 0, wide.as_ptr(), wide.len() as i32,
        bytes.as_mut_ptr(), size, std::ptr::null(), flag) };
    assert_eq!(written, size);
    (bytes, used != 0)
}
pub(super) fn memory_probe() {
    let samples = ["轻述远程输入测试 RDP 0123", "中文 English 你好", "🙂 𠮷 é",
        "line1\r\n第二行", r"C:\work\中文\file.txt"];
    for (index, text) in samples.iter().enumerate() {
        // Actual production allocation/locking and exact buffer, not a replica.
        let global = text_memory(text).expect("fixed text allocation");
        let expected: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
        unsafe {
            let bytes = GlobalSize(global.0);
            assert!(bytes >= expected.len() * 2 && bytes <= MAX_TEXT_BYTES);
            let lock = MemoryLock::new(global.0).expect("lock allocated memory");
            let actual = std::slice::from_raw_parts(lock.ptr.cast::<u16>(), expected.len());
            assert_eq!(actual, expected, "production UTF-16 payload changed");
            let all = std::slice::from_raw_parts(lock.ptr, bytes);
            assert!(all[expected.len() * 2..].iter().all(|&byte| byte == 0));
        }
        let (utf8, replacement) = converted(text, 65001);
        assert_eq!(utf8, text.as_bytes());
        assert!(!replacement);
        println!("sample={} production_utf16_exact=true utf8_exact=true", index);
    }
    assert!(matches!(text_memory(""), Err("manual_paste_invalid_text")));
    assert!(matches!(text_memory("safe\0suffix"), Err("manual_paste_invalid_text")));
    let (ansi, replacement) = converted(samples[0], 1252);
    assert!(replacement, "CP1252 cannot represent the Chinese sample");
    assert_eq!(ansi, b"???????? RDP 0123");
    println!("forced_cp1252 chinese_questions=8 ascii_preserved=true replacement_detected=true");
    println!("native_text_memory=passed samples=5 clipboard_api_calls=0 key_injection=0 rdp_transport_tested=false");
}
'''


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--windows-rlib", type=Path)
    parser.add_argument("--native-text-memory", action="store_true")
    parser.add_argument("--build-manual-rdp-probe", type=Path,
                        help="build only; fixed-text clipboard probe requires separate explicit invocation")
    parser.add_argument("--build-automatic-rdp-probe", type=Path,
                        help="build only; one separately authorized fixed-input attempt to foreground MSTSC")
    parser.add_argument("--enigo-rlib", type=Path)
    args = parser.parse_args()
    if args.windows_rlib and os.name != "nt":
        parser.error("--windows-rlib requires a Windows host")
    if args.native_text_memory and not args.windows_rlib:
        parser.error("--native-text-memory requires --windows-rlib")
    if args.build_manual_rdp_probe and not args.windows_rlib:
        parser.error("--build-manual-rdp-probe requires --windows-rlib")
    if args.build_automatic_rdp_probe and (not args.windows_rlib or not args.enigo_rlib):
        parser.error("--build-automatic-rdp-probe requires --windows-rlib and --enigo-rlib")
    if args.build_manual_rdp_probe and args.build_automatic_rdp_probe:
        parser.error("build one probe kind at a time")
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
            if args.build_manual_rdp_probe or args.build_automatic_rdp_probe:
                policy = (root / "desktop/src-tauri/src/protected_paste_policy.rs").as_posix()
                native = (root / "desktop/src-tauri/src/protected_paste_windows.rs").read_text(encoding="utf-8")
                start = native.index("    fn prepare(&mut self, text: &str)")
                end = native.index("        self.held = Some(ClipboardLock::open", start)
                prefix = native[start:end].replace("fn prepare(", "fn prepare_fixed(", 1)
                assert "GetClipboardData" not in prefix and "snapshot()" not in prefix
                tail = (root / "scripts/manual-rdp-text-probe.rs").read_text(encoding="utf-8")
                assert tail.count("// PRODUCTION_ALLOCATIONS") == 1
                tail = tail.replace("// PRODUCTION_ALLOCATIONS", prefix[prefix.index("{") + 1:])
                header = ''
                automatic = ''
                extra_link = []
                auto_hashes = {}
                if args.build_automatic_rdp_probe:
                    enigo_rlib = args.enigo_rlib.resolve(strict=True)
                    auto_hashes["enigo_rlib"] = hashlib.sha256(enigo_rlib.read_bytes()).hexdigest()
                    extra_link = ["--extern", f"enigo={enigo_rlib}", "-L", f"dependency={enigo_rlib.parent}"]
                    for name in ["manual_paste_keyboard", "keyboard_state_policy", "keyboard_state_windows"]:
                        filename = root / f"desktop/src-tauri/src/{name}.rs"
                        alias = "keyboard" if name == "manual_paste_keyboard" else name
                        header += f'#[path = "{filename.as_posix()}"] mod {alias};\n'
                        auto_hashes[name] = hashlib.sha256(filename.read_bytes()).hexdigest()
                    manual = (root / "desktop/src-tauri/src/manual_paste.rs").read_text(encoding="utf-8")
                    adapter_start = manual.index("    struct Adapter<'a>")
                    adapter_end = manual.index("    keyboard::inject_if_released(", adapter_start)
                    adapter = manual[adapter_start:adapter_end]
                    automatic = (root / "scripts/automatic-rdp-text-probe.rs").read_text(encoding="utf-8")
                    assert automatic.count("// PRODUCTION_ENIGO_ADAPTER") == 1
                    automatic = automatic.replace("// PRODUCTION_ENIGO_ADAPTER", adapter)
                    auto_hashes["enigo_adapter"] = hashlib.sha256(adapter.encode()).hexdigest()
                    auto_hashes["expanded_automatic_probe"] = hashlib.sha256(automatic.encode()).hexdigest()
                source = ('#![allow(dead_code)]\n' + f'#[path = "{policy}"] mod policy;\n'
                          + header + 'mod platform {\n' + native + '\n' + tail + automatic + '\n}\n'
                          + r'''fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = match args.as_slice() {
        [one] if one == "--manual-rdp-delayed" => false,
        [one] if one == "--manual-rdp-materialized" => true,
        _ => { println!("NO native APIs called. Explicit fixed-text clipboard test mode required.");
               return std::process::ExitCode::from(2); }
    };
    match platform::manual_probe(mode) {
        Ok(()) => { println!("status=settled fixed_baseline_settlement_requested=true remote_insertion_requires_user_confirmation=true"); std::process::ExitCode::SUCCESS }
        Err(code) => { println!("status=ended code={}", code); std::process::ExitCode::FAILURE }
    }
}
''')
                if args.build_automatic_rdp_probe:
                    source = source[:source.index("fn main() -> std::process::ExitCode")] + r'''fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (retain, scancode, unicode, pid) = match args.as_slice() {
        [mode, pid] if mode == "--automatic-rdp-restore" || mode == "--automatic-rdp-retain" || mode == "--automatic-rdp-scancode" || mode == "--automatic-rdp-unicode" => {
            match pid.parse::<u32>() {
                Ok(pid) if pid != 0 => (mode == "--automatic-rdp-retain", mode == "--automatic-rdp-scancode", mode == "--automatic-rdp-unicode", pid),
                _ => { println!("NO native APIs called. Positive target PID required."); return std::process::ExitCode::from(2); }
            }
        },
        _ => { println!("NO native APIs called. Explicit one-shot RDP mode and PID required."); return std::process::ExitCode::from(2); }
    };
    match platform::automatic_probe(pid, retain, scancode, unicode) {
        Ok(()) => { println!("status=settled remote_insertion_requires_user_confirmation=true"); std::process::ExitCode::SUCCESS },
        Err(code) => { println!("status=ended code={}", code); std::process::ExitCode::FAILURE },
    }
}
'''
                probe = work / "manual_rdp_probe.rs"
                probe.write_text(source, encoding="utf-8")
                binary = (args.build_automatic_rdp_probe or args.build_manual_rdp_probe).resolve()
                binary.parent.mkdir(parents=True, exist_ok=True)
                subprocess.run([rustc, "--edition=2021", str(probe), "--extern", f"windows={rlib}",
                    "-L", f"dependency={rlib.parent}", *extra_link, "-o", str(binary)], check=True)
                binary.with_suffix(".generated.rs").write_text(source, encoding="utf-8")
                binary.with_suffix(".manifest.json").write_text(json.dumps({
                    "built_only": True, "automatic_keys": bool(args.build_automatic_rdp_probe),
                    "automatic_probe_hashes": auto_hashes,
                    "reads_existing_clipboard_payload": False, "requires_explicit_test_consent": True,
                    "source_sha256": hashlib.sha256(source.encode()).hexdigest(),
                    "production_windows_sha256": hashlib.sha256(native.encode()).hexdigest(),
                    "policy_sha256": hashlib.sha256(Path(policy).read_bytes()).hexdigest(),
                    "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                    "tail_sha256": hashlib.sha256(tail.encode()).hexdigest(),
                }, indent=2), encoding="utf-8")
                print(f"Fixed-text probe BUILT ONLY: {binary}. No clipboard or keyboard APIs executed.")
            if args.native_text_memory:
                policy = (root / "desktop/src-tauri/src/protected_paste_policy.rs").as_posix()
                native = (root / "desktop/src-tauri/src/protected_paste_windows.rs").read_text(encoding="utf-8")
                probe = work / "text_memory_probe.rs"
                probe.write_text('#![allow(dead_code)]\n' + f'#[path = "{policy}"] mod policy;\n'
                    + 'mod platform {\n' + native + '\n' + TEXT_MEMORY_PROBE + '\n}\n'
                    + 'fn main() { platform::memory_probe(); }\n', encoding="utf-8")
                binary = work / "text_memory_probe.exe"
                subprocess.run([rustc, "--edition=2021", str(probe), "--extern", f"windows={rlib}",
                    "-L", f"dependency={rlib.parent}", "-o", str(binary)], check=True)
                subprocess.run([str(binary)], check=True, timeout=10)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
