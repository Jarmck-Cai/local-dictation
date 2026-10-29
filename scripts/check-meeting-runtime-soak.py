"""Real-time synthetic CaptureFactory -> production queue/actor/store/stop soak.

Uses cached Rust dependencies; no devices, microphone, playback, models or user Data.
Test-only observation adapters measure queue reservation and successful appends.
"""
from pathlib import Path
import argparse
import hashlib
import json
import shutil
import subprocess
import time

def main():
    repo = Path(__file__).resolve().parents[1]
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--root', type=Path)
    p.add_argument('--seconds', type=int, default=5)
    p.add_argument('--readers', type=int, default=2)
    p.add_argument('--deps', type=Path, required=True)
    p.add_argument('--output-dir', type=Path, default=repo/'outputs/runtime-soak-build')
    p.add_argument('--build-only', action='store_true')
    args = p.parse_args()
    out = args.output_dir.resolve(); out.mkdir(parents=True, exist_ok=True)
    modules = repo/'desktop/src-tauri/src/meetings'
    sources = list(modules.glob('*.rs')) + [repo/'scripts/meeting-runtime-soak.rs', Path(__file__).resolve()]
    snapshots = {s: s.read_bytes() for s in sources}
    hashes = {str(s.relative_to(repo)): hashlib.sha256(data).hexdigest() for s, data in snapshots.items()}
    frozen = out/'frozen'; frozen_modules = frozen/'meetings'
    frozen_modules.mkdir(parents=True, exist_ok=True)
    for source_path, data in snapshots.items():
        if source_path.parent == modules:
            if source_path.stem in ('types','contracts'):
                data = data.replace(b', specta::Type', b'')
            (frozen_modules/source_path.name).write_bytes(data)
    (frozen/'meeting-runtime-soak.rs').write_bytes(snapshots[repo/'scripts/meeting-runtime-soak.rs'])
    source = '#![allow(dead_code,unused_imports)]\nmod meetings {\n' + ''.join(
        f'#[path="{(frozen_modules/f"{n}.rs").as_posix()}"] pub mod {n};\n'
        for n in ('types','contracts','queue','store','alignment','runtime')) + '''
pub mod capture {
 use super::{queue::CaptureSender, types::CaptureConfig}; use std::sync::{Arc,atomic::AtomicBool};
 pub struct CaptureHandle;
 impl CaptureHandle {pub fn start_qpc_100ns(&self)->u64{0} pub fn stop(self)->Result<u64,String>{Err("synthetic_only".into())}}
 pub fn start_capture(_:CaptureConfig,_:CaptureSender,_:Arc<AtomicBool>)->Result<CaptureHandle,String>{Err("synthetic_only".into())}
}
}\n''' + f'include!("{(frozen/"meeting-runtime-soak.rs").as_posix()}");\n'
    harness = out/'soak.rs'; harness.write_text(source,encoding='utf-8')
    exe = out/'meeting-runtime-soak.exe'
    cmd = [shutil.which('rustc') or str(Path.home()/'.cargo/bin/rustc.exe'),'--edition=2021','--cfg','test','-O',str(harness),'-L',f'dependency={args.deps.resolve()}','-o',str(exe)]
    for n in ('serde','serde_json','rubato','hound'):
        candidates=sorted(args.deps.glob(f'lib{n}-*.rlib'),key=lambda f:f.stat().st_mtime,reverse=True)
        if not candidates: p.error(f'Missing immutable dependency: {n}')
        cmd += ['--extern',f'{n}={candidates[0]}']
    subprocess.run(cmd,check=True)
    assert all(s.read_bytes() == data for s, data in snapshots.items()), 'Source changed during freeze/build; discard this candidate'
    receipt={'created_unix':time.time(),'source_sha256':hashes,'frozen_directory':str(frozen),'executable_sha256':hashlib.sha256(exe.read_bytes()).hexdigest(),'compiler':subprocess.check_output([cmd[0],'--version'],text=True).strip(),'command':cmd}
    (out/'source-freeze.json').write_text(json.dumps(receipt,indent=2),encoding='utf-8')
    if args.build_only: print(exe); return 0
    root=(args.root or out/f'run-{time.time_ns()}').resolve()
    command=[str(exe),'--root',str(root),'--seconds',str(args.seconds),'--readers',str(args.readers)]
    print(json.dumps({'phase':'start','root':str(root),'command':command,'source_freeze':str(out/'source-freeze.json')}),flush=True)
    return subprocess.run(command,cwd=out).returncode

if __name__=='__main__': raise SystemExit(main())
