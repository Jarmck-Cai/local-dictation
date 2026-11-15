#!/usr/bin/env python3
"""Build the real production SpeexDSP wrapper and apply frozen synthetic gates.

Requires numpy, rustc, MSVC x64 tools and eSpeak NG. Speech is generated to WAV;
this script never opens an audio device or plays/records any sound. All generated
audio, intermediate binaries and metric reports stay in ignored outputs/.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import subprocess
import time
import wave

import numpy as np

RATE = 16_000
FRAME = 160
ROOT = Path(__file__).resolve().parents[1]
ARCHIVE_SHA256 = "8c777343e4a6399569c72abc38a95b24db56882c83dbdb6c6424a5f4aeb54d3d"


def command(args, **kwargs):
    subprocess.run([str(a) for a in args], check=True, **kwargs)


def native_build(out):
    locator = Path(os.environ.get("ProgramFiles(x86)", "C:/Program Files (x86)")) / "Microsoft Visual Studio/Installer/vswhere.exe"
    installation = subprocess.check_output([str(locator), "-latest", "-products", "*", "-requires", "Microsoft.VisualStudio.Component.VC.Tools.x86.x64", "-property", "installationPath"], text=True).strip()
    if not installation:
        raise RuntimeError("MSVC x64 C tools not found")
    vcvars = Path(installation) / "VC/Auxiliary/Build/vcvars64.bat"
    vendor = ROOT / "desktop/src-tauri/native/speexdsp-1.2.1"
    # Paths are locally discovered, never user-supplied shell fragments.
    lines = ["@echo off", f'call "{vcvars}" >nul', "if errorlevel 1 exit /b 1"]
    files = " ".join(f'"{vendor / "libspeexdsp" / name}"' for name in ("mdf.c", "fftwrap.c", "smallft.c"))
    lines += [f'cl /nologo /c /TC /O2 /MD /DFLOATING_POINT /DUSE_SMALLFT /DEXPORT= /Dinline=__inline /I"{vendor / "include"}" /I"{vendor / "libspeexdsp"}" {files}', "if errorlevel 1 exit /b 1", "lib /nologo /out:meeting_speexdsp.lib mdf.obj fftwrap.obj smallft.obj", "if errorlevel 1 exit /b 1"]
    build = out / "build-native.cmd"
    build.write_text("\r\n".join(lines) + "\r\n", encoding="ascii")
    command(["cmd.exe", "/d", "/c", str(build)], cwd=out)


def rust_build(out):
    module = ROOT / "desktop/src-tauri/src/meetings/aec.rs"
    # The probe imports the production wrapper; it does not reimplement AEC.
    source = r'''#[path = "MODULE"] mod aec;
use std::{fs::File, io::{Read,Write,BufReader,BufWriter}, env};
fn main() {
    let args: Vec<_> = env::args().collect();
    let mut input=BufReader::new(File::open(&args[1]).unwrap());
    let mut output=BufWriter::new(File::create(&args[2]).unwrap());
    let mut state=aec::MeetingAec::new().unwrap();
    let mut bytes=[0u8;1280]; let mut near=[0f32;160];
    let mut reference=[0f32;160]; let mut cleaned=[0f32;160];
    loop {
        let mut first=[0u8;1];
        if input.read(&mut first).unwrap()==0 { break; }
        bytes[0]=first[0]; input.read_exact(&mut bytes[1..]).unwrap();
        for i in 0..160 {
            near[i]=f32::from_le_bytes(bytes[i*4..i*4+4].try_into().unwrap());
            reference[i]=f32::from_le_bytes(bytes[640+i*4..644+i*4].try_into().unwrap());
        }
        state.process(&near,&reference,&mut cleaned).unwrap();
        for sample in cleaned { output.write_all(&sample.to_le_bytes()).unwrap(); }
    }
    output.flush().unwrap();
}'''.replace("MODULE", module.as_posix())
    entry = out / "aec-probe.rs"
    entry.write_text(source, encoding="utf-8")
    command(["rustc", "--edition", "2021", "-O", "-A", "dead_code", str(entry), "-L", f"native={out}", "-o", out / "aec-probe.exe"])
    command(["rustc", "--edition", "2021", "--test", str(module), "-L", f"native={out}", "-o", out / "aec-tests.exe"])


def bandlimited_sample(x, positions):
    """64-tap windowed sinc; rate conversion is fixture code, not the AEC."""
    base = np.floor(positions).astype(np.int64)
    step = float(positions[1] - positions[0]) if len(positions) > 1 else 1.0
    cutoff = min(1.0, 1.0 / step)
    result = np.zeros(len(positions))
    weight_sum = np.zeros(len(positions))
    for tap in range(-31, 33):
        index = base + tap
        distance = index - positions
        window = .42 + .5 * np.cos(np.pi * distance / 32) + .08 * np.cos(2 * np.pi * distance / 32)
        weight = cutoff * np.sinc(cutoff * distance) * window
        valid = (index >= 0) & (index < len(x))
        result[valid] += weight[valid] * x[index[valid]]
        weight_sum += weight
    return result / weight_sum


def speech(executable, out, name, voice, pitch, speed, text):
    destination = out / f"{name}.wav"
    command([executable, "-v", voice, "-p", str(pitch), "-s", str(speed), "-w", destination, text])
    with wave.open(str(destination), "rb") as wav:
        assert wav.getnchannels() == 1 and wav.getsampwidth() == 2
        rate = wav.getframerate()
        x = np.frombuffer(wav.readframes(wav.getnframes()), dtype="<i2").astype(np.float64) / 32768.0
    # Deliberately noninteger 22050->16000 ratio on the installed eSpeak voice.
    t = np.arange(math.floor(len(x) * RATE / rate), dtype=np.float64) * rate / RATE
    x = bandlimited_sample(x, t)
    x -= x.mean()
    # Fixed low level also leaves headroom for the optional +10 dB near case.
    x *= 0.01 / np.sqrt(np.mean(x * x))
    return x, {"voice": voice, "pitch": pitch, "speed": speed, "source_rate": rate, "normalized_rms": math.sqrt(power(x)), "normalized_peak": float(np.max(np.abs(x))), "wav_sha256": hashlib.sha256(destination.read_bytes()).hexdigest(), "text": text}


def tiled(x, count, offset):
    return x[(np.arange(count) + offset) % len(x)]


def fixture(far_voice, near_voice, delay_ms, ppm, aligned, near_db):
    count = 42 * RATE
    far = tiled(far_voice, count, 317)
    near = tiled(near_voice, count, 1703) * 10.0 ** (near_db / 20.0)
    far[12 * RATE:20 * RATE] = 0
    far[40 * RATE:] = 0
    near[:12 * RATE] = 0
    near[32 * RATE:] = 0
    echo = np.zeros(count)
    # Fixed independent room impulse response, no adaptive/gate-derived taps.
    for extra_ms, gain in [(0, .55), (7, .20), (19, -.13), (43, .08), (81, .04)]:
        lag = round((delay_ms + extra_ms) * RATE / 1000)
        if lag:
            echo[lag:] += gain * far[:-lag]
        else:
            echo += gain * far
    scale = 1.0 + ppm / 1_000_000.0
    positions = np.arange(count, dtype=np.float64)
    raw_reference = bandlimited_sample(far, positions / scale) if ppm else far.copy()
    # Timestamp-derived device rate correction, with truth supplied by this
    # synthetic source. This is NOT evidence of the production QPC aligner.
    reference = bandlimited_sample(raw_reference, positions * scale) if aligned and ppm else raw_reference
    microphone = near + echo
    assert np.max(np.abs(microphone)) <= 1 and np.max(np.abs(reference)) <= 1, "fixture clipping invalidates test"
    return microphone, reference, near, echo


def power(x):
    return float(np.mean(np.square(x)))


def correlation(a, b):
    pa, pb = power(a), power(b)
    return float(np.mean(a * b) / math.sqrt(pa * pb)) if pa > 1e-15 and pb > 1e-15 else 0.0


def db_ratio(numerator, denominator):
    return 10 * math.log10(max(numerator, 1e-18) / max(denominator, 1e-18))


def active_mask(truth):
    frames = truth.reshape(-1, FRAME)
    return np.repeat(np.mean(frames * frames, axis=1) >= 1e-7, FRAME)


def judge(output, near, echo, matched_near=None):
    assert len(output) == len(near) == len(echo)
    assert np.isfinite(output).all()
    er = slice(35 * RATE, 40 * RATE)
    ns = slice(13 * RATE, 20 * RATE)
    dt = slice(21 * RATE, 32 * RATE)
    emask, nmask = active_mask(echo[er]), active_mask(near[ns])
    dmask = active_mask(near[dt]) & active_mask(echo[dt])
    n, y = near[ns][nmask], output[ns][nmask]
    dn, de, dy = near[dt][dmask], echo[dt][dmask], output[dt][dmask]
    near_projection = float(np.dot(dn, dy) / np.dot(dn, dn))
    residual = dy - near_projection * dn
    metrics = {
        "tail_erle_db": db_ratio(power(echo[er][emask]), power(output[er][emask])),
        "near_correlation": correlation(n, y),
        "near_energy_db": db_ratio(power(y), power(n)),
        "double_correlation": correlation(dn, dy),
        "double_energy_db": db_ratio(power(dy), power(dn)),
        # Conservative oracle residual includes waveform distortion as well as
        # uncancelled echo; a projection cannot hide shifted/filtered echo.
        "double_far_residual_reduction_db": db_ratio(power(de), power(residual)),
        "double_near_projection_gain_db": 20 * math.log10(max(abs(near_projection), 1e-12)),
        "silence_rms": math.sqrt(power(output[41 * RATE:])),
    }
    checks = {
        "echo": metrics["tail_erle_db"] >= 15.0,
        "near_correlation": metrics["near_correlation"] >= 0.99,
        "near_energy": abs(metrics["near_energy_db"]) <= 1.0,
        "double_correlation": metrics["double_correlation"] >= 0.8,
        "double_energy": abs(metrics["double_energy_db"]) <= 3.0,
        "double_echo": metrics["double_far_residual_reduction_db"] >= 10.0,
        "silence": metrics["silence_rms"] <= 1 / 32768.0,
    }
    result = {"pass": all(checks.values()), "checks": checks, "metrics": metrics}
    if matched_near is not None:
        control = matched_near[dt][dmask]
        matched_projection = float(np.dot(control, dy) / np.dot(control, control))
        matched_residual = dy - matched_projection * control
        result["matched_near_diagnostics"] = {
            "control": "same dry-near waveform through fresh upstream AEC with identically zero reference",
            "double_correlation": correlation(control, dy),
            "double_energy_db": db_ratio(power(dy), power(control)),
            "double_far_residual_reduction_db": db_ratio(power(de), power(matched_residual)),
            "double_unscaled_residual_reduction_db": db_ratio(power(de), power(dy - control)),
            "double_near_projection_gain_db": 20 * math.log10(max(abs(matched_projection), 1e-12)),
            "near_only_correlation": correlation(matched_near[ns][nmask], y),
            "near_only_energy_db": db_ratio(power(y), power(matched_near[ns][nmask])),
        }
    return result


def process(out, near, reference):
    source = out / "probe-input.f32"
    destination = out / "probe-output.f32"
    np.stack((near.reshape(-1, FRAME), reference.reshape(-1, FRAME)), axis=1).astype("<f4").tofile(source)
    started = time.perf_counter()
    command([out / "aec-probe.exe", source, destination])
    elapsed = time.perf_counter() - started
    return np.fromfile(destination, dtype="<f4").astype(np.float64), elapsed


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=ROOT / "outputs/022-meeting/aec")
    parser.add_argument("--espeak", type=Path, default=Path("C:/Program Files/eSpeak NG/espeak-ng.exe"))
    parser.add_argument("--reuse-native", action="store_true")
    parser.add_argument("--diagnostic", action="store_true", help="Write full report even if unit tests fail; exit still fails for failed gates")
    parser.add_argument("--extended", action="store_true", help="Also measure near/echo -10 and +10 dB near scaling")
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    if not args.reuse_native:
        native_build(out)
    rust_build(out)
    unit = subprocess.run([str(out / "aec-tests.exe"), "--nocapture"], text=True, capture_output=True)
    (out / "unit-tests.log").write_text(unit.stdout + unit.stderr, encoding="utf-8")
    print(unit.stdout + unit.stderr, flush=True)
    if unit.returncode and not args.diagnostic:
        raise SystemExit(unit.returncode)
    far, far_meta = speech(args.espeak, out, "far-tts", "en-us+m3", 42, 151, "The remote speaker explains a careful engineering experiment. We compare two independent clocks and preserve every saved audio frame. A quiet interval is followed by a new sentence about the library and the garden.")
    near, near_meta = speech(args.espeak, out, "near-tts", "en-gb+f3", 72, 173, "I am speaking from the local microphone while another person continues talking. Please keep the entire sentence and all of its quiet words. Tomorrow we will measure the river and bring a yellow notebook to the meeting.")
    report = {"pass": False, "complete": False, "kind": "synthetic_offline_no_playback", "backend": "SpeexDSP 1.2.1 MDF + SmallFT floating point", "archive_sha256": ARCHIVE_SHA256, "rate": RATE, "frame_samples": FRAME, "filter_samples": 5120, "native_library_bytes": (out / "meeting_speexdsp.lib").stat().st_size, "probe_exe_bytes": (out / "aec-probe.exe").stat().st_size, "unit_tests_pass": unit.returncode == 0, "fixtures": {"far": far_meta, "near": near_meta}, "scenarios": [], "uncompensated_drift_diagnostics": []}
    report["notch_source"] = {"file": "desktop/src-tauri/native/speexdsp-1.2.1/libspeexdsp/mdf.c", "function": "filter_dc_notch16", "radius_at_16k": .982, "denominator_coefficient": ".982^2 + .7*(1-.982)^2", "transfer_function": "H(z)=r*(1-2*z^-1+z^-2)/(1-2*r*z^-1+(r^2+.7*(1-r)^2)*z^-2)", "api_latency_samples": 0, "official_source": "https://github.com/xiph/speexdsp/blob/SpeexDSP-1.2.1/libspeexdsp/mdf.c"}
    report["source_sha256"] = {str(path.relative_to(ROOT)).replace("\\", "/"): hashlib.sha256(path.read_bytes()).hexdigest() for path in [ROOT / "desktop/src-tauri/src/meetings/aec.rs", ROOT / "scripts/check-meeting-aec.py"] + sorted((ROOT / "desktop/src-tauri/native/speexdsp-1.2.1").rglob("*.c"))}
    matched_controls = {}
    for level in ((-10,0,10) if args.extended else (0,)):
        _, _, control_near, _ = fixture(far,near,0,0,True,level)
        matched_controls[level], _ = process(out, control_near, np.zeros_like(control_near))
    scenarios = [(delay, ppm, 0) for delay in (0, 40, 80, 120) for ppm in (0, -100, 100)]
    if args.extended:
        scenarios += [(delay, 0, near_db) for delay in (0,40,80,120) for near_db in (-10,10)]
    for delay, ppm, near_db in scenarios:
        mic, reference, n, echo = fixture(far, near, delay, ppm, True, near_db)
        cleaned, elapsed = process(out, mic, reference)
        result = judge(cleaned, n, echo, matched_controls[near_db])
        result.update(delay_ms=delay, drift_ppm=ppm, aligned=True, near_scale_db=near_db, processing_seconds=elapsed)
        report["scenarios"].append(result)
        (out / "aec-report.json").write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
        print(json.dumps(result, sort_keys=True), flush=True)
        if delay == 40 and ppm == 0 and near_db == 0:
            zero = judge(np.zeros_like(n), n, echo)
            passthrough = judge(mic, n, echo)
            report["negative_controls"] = {"zero": zero, "passthrough": passthrough}
            assert not zero["pass"] and not passthrough["pass"], "gate falsely accepts zero/passthrough"
            # Diagnostic lag search only. Gate above always uses the actual zero
            # sample API latency. Optimizing voice lag cannot alter the verdict.
            ns = slice(13 * RATE, 20 * RATE)
            report["near_lag_diagnostic"] = {str(lag): correlation(n[ns][320:-320], cleaned[13*RATE+320+lag:20*RATE-320+lag]) for lag in range(-16,17)}
            spectrum = np.abs(np.fft.rfft(n[ns])) ** 2
            frequencies = np.fft.rfftfreq(len(n[ns]), 1/RATE)
            report["near_spectrum_power_fraction"] = {str(hz): float(spectrum[frequencies < hz].sum()/spectrum.sum()) for hz in (50,100,200,500)}
    for ppm in (-100,100):
        mic, reference, n, echo = fixture(far, near, 40, ppm, False, 0)
        cleaned, elapsed = process(out, mic, reference)
        result = judge(cleaned,n,echo,matched_controls[0])
        result.update(delay_ms=40, drift_ppm=ppm, aligned=False, processing_seconds=elapsed)
        report["uncompensated_drift_diagnostics"].append(result)
    report["pass"] = unit.returncode == 0 and all(s["pass"] for s in report["scenarios"])
    report["complete"] = True
    (out / "aec-report.json").write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"REPORT {out / 'aec-report.json'} pass={report['pass']}", flush=True)
    raise SystemExit(0 if report["pass"] else 1)


if __name__ == "__main__":
    main()
