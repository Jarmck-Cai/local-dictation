"""Short production meeting processing + actual SenseVoice check, synthetic WAV only.

Pass a newly synthesized 16kHz mono PCM16 fixture, never an existing meeting.
No devices, playback, cloud, app settings or installed user Data are accessed.
"""
from pathlib import Path
import argparse
import hashlib
import json
import shutil
import subprocess

def main():
    repo=Path(__file__).resolve().parents[1]
    p=argparse.ArgumentParser(description=__doc__)
    for name in ('deps','native','ort','wrapper','model','vad','wav','output-dir'):
        p.add_argument('--'+name,type=Path,required=True)
    a=p.parse_args(); out=a.output_dir.resolve(); out.mkdir(parents=True,exist_ok=True)
    modules=repo/'desktop/src-tauri/src/meetings'
    for name in ('types','contracts'):
        (out/f'{name}.rs').write_text((modules/f'{name}.rs').read_text(encoding='utf-8').replace(', specta::Type',''),encoding='utf-8')
    source='#![allow(dead_code)]\nmod meetings {\n'+''.join(f'#[path="{((out if n in ("types","contracts") else modules)/f"{n}.rs").as_posix()}"] pub mod {n};\n' for n in ('types','contracts','queue','store','alignment','apm','processing'))+'}\n'+r'''
use meetings::{store::MeetingStore,types::*};
use transcribe_rs::onnx::{sense_voice::{SenseVoiceModel,SenseVoiceParams},Quantization};
use std::{path::Path,sync::atomic::AtomicBool};
fn main() {
 let args:Vec<_>=std::env::args().collect();
 let mut wav=hound::WavReader::open(&args[1]).unwrap();
 assert_eq!(wav.spec().sample_rate,16000);assert_eq!(wav.spec().channels,1);assert_eq!(wav.spec().bits_per_sample,16);
 let mut samples=vec![0f32;16000]; samples.extend(wav.samples::<i16>().map(|s|s.unwrap() as f32/32768.0));samples.extend(vec![0f32;16000]);
 let store=MeetingStore::new(Path::new(&args[4]).join("synthetic-meetings")).unwrap();
 let mut writer=store.create("synthetic-speech","Controlled generated speech",10_000_000,vec![]).unwrap();
 for (sequence,part) in samples.chunks(320).enumerate() {
  for source in [Source::Microphone,Source::Remote] {
   writer.heartbeat(source,0,10_000_000+sequence as u64*200_000).unwrap();
   let data=part.iter().flat_map(|s|if source==Source::Remote {s.to_le_bytes()} else {0f32.to_le_bytes()}).collect();
   writer.append(CapturePacket {source,epoch:0,sequence:sequence as u64,device_frame_position:sequence as u64*320,qpc_100ns:10_000_000+sequence as u64*200_000,frames:part.len() as u32,flags:0,
     format:AudioFormat {sample_rate:16000,channels:1,bits_per_sample:32,valid_bits:32,channel_mask:4,encoding:AudioEncoding::Float},data}).unwrap();
  }
 }
 writer.finish(MeetingStatus::Interrupted,Some(10_000_000+samples.len() as u64*625),Some("meeting_io".into())).unwrap();
 let began=std::time::Instant::now();let mut model=SenseVoiceModel::load(Path::new(&args[2]),&Quantization::Int8).unwrap();
 let mut calls=0;
 let transcript=meetings::processing::process_meeting(&store,"synthetic-speech",Path::new(&args[3]),&AtomicBool::new(false),|audio| {
  calls+=1;model.transcribe_with(&audio,&SenseVoiceParams {language:Some("en".into()),use_itn:Some(true)}).map(|r|r.text).map_err(|_|"synthetic_asr_failed".into())
 },|_|{}).unwrap();
 assert!(transcript.complete);assert!(calls>0);assert!(transcript.segments.iter().any(|s|s.source==Source::Remote&&!s.text.trim().is_empty()));
 let text=transcript.segments.iter().map(|s|s.text.to_lowercase()).collect::<Vec<_>>().join(" ");assert!(text.contains("recording"));
 let m=store.load("synthetic-speech").unwrap();assert_eq!(m.status,MeetingStatus::Complete);assert_eq!(m.capture_error.as_deref(),Some("meeting_io"));
 println!("{}",serde_json::json!({"actual_asr":true,"production_processing":true,"synthetic_only":true,"calls":calls,"segments":transcript.segments.len(),"capture_error":m.capture_error,"elapsed_seconds":began.elapsed().as_secs_f64(),"synthetic_text":text}));
}
'''
    harness=out/'asr.rs';harness.write_text(source,encoding='utf-8')
    exe=out/'synthetic-asr.exe'
    cmd=[shutil.which('rustc') or str(Path.home()/'.cargo/bin/rustc.exe'),'--edition=2021','-O',str(harness),'-L',f'dependency={a.deps.resolve()}','-L',f'native={a.native.resolve()}','-L',f'native={a.ort.resolve()}','-C',f'link-arg={a.wrapper.resolve()}','-o',str(exe)]
    for name in ('serde','serde_json','rubato','vad_rs','transcribe_rs','hound'):
        libs=sorted(a.deps.glob(f'lib{name}-*.rlib'),key=lambda f:f.stat().st_mtime,reverse=True)
        if not libs: p.error(f'Missing immutable dependency: {name}')
        cmd+=['--extern',f'{name}={libs[0]}']
    subprocess.run(cmd,check=True)
    shutil.copy2(a.ort/'onnxruntime.dll',out/'onnxruntime.dll')
    print(json.dumps({'fixture_sha256':hashlib.sha256(a.wav.read_bytes()).hexdigest(),'model_sha256':hashlib.sha256((a.model/'model.int8.onnx').read_bytes()).hexdigest(),'executable_sha256':hashlib.sha256(exe.read_bytes()).hexdigest()}),flush=True)
    return subprocess.run([str(exe),str(a.wav.resolve()),str(a.model.resolve()),str(a.vad.resolve()),str(out)],cwd=out).returncode

if __name__=='__main__':raise SystemExit(main())
