#!/usr/bin/env python3
"""Frozen, independent H-reference AEC v2 gates; no devices or playback.

Run with numpy, MSVC, rustc and eSpeak NG. A standalone native library is built;
the real production Rust wrapper is imported without using shared Cargo targets.
The original v1 script/report are never modified. --extended adds held-out full
utterances, rooms and fixed stress cases; results never tune the implementation.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import importlib.util
import json
import math
from pathlib import Path
import subprocess
import time

import numpy as np

ROOT = Path(__file__).resolve().parents[1]
RATE, FRAME, RADIUS = 16000, 160, .982
spec = importlib.util.spec_from_file_location("aec_v1_frozen", ROOT / "scripts/check-meeting-aec.py")
v1 = importlib.util.module_from_spec(spec)
spec.loader.exec_module(v1)

# Protocol is written before native build or observing any new output. These
# constants are engineering requirements, never estimated from the test result.
PROTOCOL = {
    "version": 2,
    "reference": "independent direct-form H from published filter_dc_notch16; no native/matched control",
    "h": {"radius": RADIUS, "b": [RADIUS, -2*RADIUS, RADIUS],
          "a": [1, -2*RADIUS, RADIUS**2 + .7*(1-RADIUS)**2],
          "input_quantization": "f32 input; multiply32768; round ties away from zero; int16 clamp",
          "output_quantization": "floor(.5+H(x)*32768), int16 clamp, divide32768",
          "state": "zero initial state; continuous entire dry-near timeline; zero sample lag"},
    "thresholds": {"echo_erle_db_min": 15, "near_h_corr_min": .99,
                   "near_dry_energy_db_abs_max": 1, "double_unscaled_h_residual_db_min": 10,
                   "double_positive_h_projection_db_abs_max": 3, "double_dry_corr_min": .8,
                   "silence_rms_max": 1/32768},
    "baseline": "unchanged v1 42s source/room/time windows; delays0/40/80/120 x drift0/-100/+100ppm; near±10dB",
    "heldout": "complete new untiled utterances; rooms B/C fixed below; delays0/40/80/120; near±10dB and calibrated SER±10dB",
    "challenge_windows": "cold double and changed-path immediate double: whole plus fixed0-1s/1-3s/3s-to-end; after3s same double gate; first0-3s positive H projection within±3dB, no zero/mute; all early suppression scores still reported",
    "exploration": "background SNR25dB, high level x4, smooth echo nonlinearity tanh(3x)/3; no threshold changes",
    "sample_selection": "10ms truth-active frames mean-square>=1e-7; double requires both near and echo; never output-derived",
    "negative_controls": ["zero", "passthrough", "fixed_hpf_only"],
    "forbidden": ["matched AEC output as truth", "gain/delay/filter fitting", "gate threshold search", "per-frame mute"],
    "official_source": "https://github.com/xiph/speexdsp/blob/SpeexDSP-1.2.1/libspeexdsp/mdf.c",
}


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def fixed_h(x, quantized=True):
    """Independent direct-form IIR, algebraic transfer function, float64 state.

    Does not copy Speex state-space code or call the tested library. Double state
    avoids reproducing native floating-point roundoff as an oracle. Known wrapper
    input and public WORD2INT output quantization are explicit and fixed.
    """
    x = np.asarray(x, dtype=np.float64)
    if quantized:
        scaled = x.astype(np.float32) * np.float32(32768)
        x = np.clip(np.copysign(np.floor(np.abs(scaled) + .5), scaled), -32768, 32767) / 32768
    b0, b1, b2 = PROTOCOL["h"]["b"]
    _, a1, a2 = PROTOCOL["h"]["a"]
    y = np.empty(len(x), dtype=np.float64)
    x1 = x2 = y1 = y2 = 0.0
    for i, sample in enumerate(x):
        value = b0*sample + b1*x1 + b2*x2 - a1*y1 - a2*y2
        y[i] = value
        x2, x1, y2, y1 = x1, sample, y1, value
    return np.clip(np.floor(y*32768 + .5), -32768, 32767) / 32768 if quantized else y


def h_self_test():
    # Analytic complex response is independent of the recurrence implementation.
    impulse = np.zeros(16384)
    impulse[0] = 1
    response = fixed_h(impulse, False)
    rows = []
    for hz in (0,20,50,100,200,500,1000,4000,7900):
        z = np.exp(-2j*np.pi*hz/RATE)
        expected = RADIUS*(1-2*z+z*z)/(1-2*RADIUS*z+(RADIUS**2+.7*(1-RADIUS)**2)*z*z)
        actual = np.sum(response*np.exp(-2j*np.pi*hz*np.arange(len(response))/RATE))
        assert abs(actual-expected) < 1e-10, (hz, actual, expected)
        rows.append({"hz": hz, "analytic_gain": abs(expected), "absolute_complex_error": abs(actual-expected)})
    assert np.array_equal(fixed_h(np.zeros(1600)), np.zeros(1600))
    assert fixed_h(np.array([.5/32768]))[0] == 1/32768
    assert fixed_h(np.array([-.5/32768]))[0] == -1/32768
    return {"pass": True, "frequency_response": rows, "quantization_and_silence_pass": True}


def selection(x, start, end):
    return x[round(start*RATE):round(end*RATE)]


def double_metrics(y, near, echo, h, start, end):
    dn, de, dy, dh = [selection(x,start,end) for x in (near,echo,y,h)]
    mask = v1.active_mask(dn) & v1.active_mask(de)
    dn, de, dy, dh = [x[mask] for x in (dn,de,dy,dh)]
    assert len(dn) > 0, "double score has no truth-active samples"
    projection = float(np.dot(dh,dy)/np.dot(dh,dh))
    m = {"dry_correlation": v1.correlation(dn,dy), "h_correlation_diagnostic": v1.correlation(dh,dy),
         "unscaled_h_residual_reduction_db": v1.db_ratio(v1.power(de),v1.power(dy-dh)),
         "h_projection_gain": projection,
         "h_projection_gain_db": 20*math.log10(max(abs(projection),1e-12)),
         "dry_total_error_reduction_db_diagnostic": v1.db_ratio(v1.power(de),v1.power(dy-dn)),
         "dry_energy_db_diagnostic": v1.db_ratio(v1.power(dy),v1.power(dn)),
         "input_ser_db": v1.db_ratio(v1.power(dn),v1.power(de)), "truth_active_samples": len(dn)}
    c = {"double_dry_correlation": m["dry_correlation"] >= .8,
         "double_positive_projection": projection > 0 and abs(m["h_projection_gain_db"]) <= 3,
         "double_unscaled_h_residual": m["unscaled_h_residual_reduction_db"] >= 10}
    return {"pass": all(c.values()), "checks": c, "metrics": m, "window_seconds": [start,end]}


def judge(y, near, echo, h, windows=None):
    assert len(y) == len(near) == len(echo) == len(h) and np.isfinite(y).all()
    windows = windows or {"near": [13,20], "double": [21,32], "echo": [35,40], "silence": [41,42]}
    n, ny, nh = [selection(x,*windows["near"]) for x in (near,y,h)]
    mask = v1.active_mask(n)
    n, ny, nh = [x[mask] for x in (n,ny,nh)]
    e, ey = [selection(x,*windows["echo"]) for x in (echo,y)]
    mask = v1.active_mask(e)
    e, ey = e[mask], ey[mask]
    dt = double_metrics(y,near,echo,h,*windows["double"])
    m = {"near_h_correlation": v1.correlation(nh,ny), "near_dry_correlation_diagnostic": v1.correlation(n,ny),
         "near_dry_energy_db": v1.db_ratio(v1.power(ny),v1.power(n)),
         "near_dry_total_error_db_diagnostic": v1.db_ratio(v1.power(ny-n),v1.power(n)),
         "tail_erle_db": v1.db_ratio(v1.power(e),v1.power(ey)),
         "silence_rms": math.sqrt(v1.power(selection(y,*windows["silence"]))) }
    c = {"near_h_correlation": m["near_h_correlation"] >= .99,
         "near_dry_energy": abs(m["near_dry_energy_db"]) <= 1,
         "echo": m["tail_erle_db"] >= 15, "silence": m["silence_rms"] <= 1/32768, **dt["checks"]}
    return {"pass": all(c.values()), "checks": c, "metrics": m, "double": dt, "windows_seconds": windows}


def provenance():
    vendor = ROOT / "desktop/src-tauri/native/speexdsp-1.2.1"
    paths = [p for p in vendor.rglob("*") if p.is_file()]
    paths += [ROOT / "desktop/src-tauri/src/meetings/aec.rs", ROOT / "desktop/src-tauri/src/meetings/aec_tests.rs",
              ROOT / "scripts/check-meeting-aec.py", Path(__file__).resolve()]
    return {p.relative_to(ROOT).as_posix(): sha(p) for p in sorted(paths)}


def save(out, report):
    (out / "aec-v2-report.json").write_text(json.dumps(report,indent=2,ensure_ascii=False)+"\n",encoding="utf-8")


def baseline(out, espeak, report):
    far, fm = v1.speech(espeak,out,"far-tts","en-us+m3",42,151,
        "The remote speaker explains a careful engineering experiment. We compare two independent clocks and preserve every saved audio frame. A quiet interval is followed by a new sentence about the library and the garden.")
    near, nm = v1.speech(espeak,out,"near-tts","en-gb+f3",72,173,
        "I am speaking from the local microphone while another person continues talking. Please keep the entire sentence and all of its quiet words. Tomorrow we will measure the river and bring a yellow notebook to the meeting.")
    report["baseline_fixtures"] = {"far": fm, "near": nm}
    refs = {level: fixed_h(v1.fixture(far,near,0,0,True,level)[2]) for level in (-10,0,10)}
    scenarios = [(d,p,0) for d in (0,40,80,120) for p in (0,-100,100)]
    scenarios += [(d,0,n) for d in (0,40,80,120) for n in (-10,10)]
    for delay, ppm, level in scenarios:
        mic, ref, n, echo = v1.fixture(far,near,delay,ppm,True,level)
        y, elapsed = v1.process(out,mic,ref)
        r = judge(y,n,echo,refs[level])
        r.update(delay_ms=delay,drift_ppm=ppm,near_scale_db=level,processing_seconds=elapsed)
        report["baseline"].append(r)
        print(f"BASE delay={delay} ppm={ppm} near={level} pass={r['pass']} Hcorr={r['metrics']['near_h_correlation']:.8f} DTres={r['double']['metrics']['unscaled_h_residual_reduction_db']:.3f}",flush=True)
        if delay == 40 and ppm == 0 and level == 0:
            report["negative_controls"] = {name: judge(value,n,echo,refs[level]) for name,value in
                {"zero": np.zeros_like(n), "passthrough": mic, "fixed_hpf_only": fixed_h(mic)}.items()}
            assert not any(r["pass"] for r in report["negative_controls"].values()), "negative accepted"
        save(out,report)


ROOMS = {
    "B": [(3,.48),(11,-.24),(27,.17),(59,.11),(107,-.065),(173,.035)],
    "C": [(4,.38),(15,.25),(35,-.19),(72,.10),(130,.06),(191,-.03)],
}


def echo_room(far, room, delay):
    echo = np.zeros(len(far))
    for ms, gain in ROOMS[room]:
        lag = round((delay+ms)*RATE/1000)
        echo[lag:] += gain*far[:-lag]
    return echo


def heldout_sources(out, espeak):
    # Each slot is a whole new utterance with leading/trailing silence retained.
    # No tiling, mid-sentence cut, voice reuse from baseline, or random offsets.
    texts = {
      "warm": ("en-sc+m1",36,157,"Before the inspection begins, the remote engineer describes the room. A blue cabinet stands beside the window and seven cardboard boxes rest underneath the table. The ventilation system is quiet. We shall compare the measurements on Tuesday morning, write down each unexpected result, and leave enough time for another careful experiment. Every recorded section must keep its original clock information."),
      "near": ("en-us+f2",66,169,"My name in this synthetic test is the local speaker. I need to preserve the quiet beginning of this complete sentence. On Wednesday the small train leaves the station at nine fifteen. Please bring three green folders, a pencil, and the updated diagram. The final number is forty seven and the final word is lighthouse."),
      "double_far": ("en-sc+m1",36,157,"The remote participant continues with a different paragraph about the observatory. A visitor discovered twelve photographs inside a wooden drawer. The oldest photograph shows a bridge after a summer storm. We plan to count the missing labels, compare the dates, and send the finished catalogue next Friday. The description ends here with the word telescope."),
      "double_near": ("en-us+f2",66,169,"I am interrupting with a complete independent statement while the remote person is talking. Our orange notebook contains six separate measurements from the northern garden. None of those values should be deleted when the voices overlap. The next appointment is on Thursday afternoon at four thirty. Please keep the final words silver bicycle."),
      "tail": ("en-sc+m1",36,157,"After the discussion the remote speaker reads the final checklist. Check the narrow corridor, close the south window, and place the spare key on the shelf. The sample container is marked twenty three. Tomorrow a different colleague will visit the laboratory and repeat this experiment without changing the agreed thresholds. This is the end of the remote statement."),
    }
    signals, metadata = {}, {}
    for name,(voice,pitch,speed,text) in texts.items():
        signals[name],metadata[name] = v1.speech(espeak,out,"heldout-"+name,voice,pitch,speed,text)
    return signals,metadata


def pad_frames(x):
    return np.pad(x,(0,(-len(x)) % FRAME))


def full_fixture(signals, delay=40, near_db=0, ser_db=None, mode="standard", stress="none"):
    lengths = {k: len(pad_frames(v)) for k,v in signals.items()}
    order = ["double","near","warm","redouble","tail","silence"] if mode == "cold" else ["warm","near","double","tail","silence"]
    phase_sizes = {"warm": max(12*RATE,lengths["warm"]), "near": lengths["near"],
                   "double": max(lengths["double_far"],lengths["double_near"]),
                   "redouble": max(lengths["double_far"],lengths["double_near"]),
                   "tail": max(12*RATE,lengths["tail"]), "silence": 2*RATE}
    total = sum(phase_sizes[k] for k in order)
    far, near = np.zeros(total),np.zeros(total)
    phases, cursor = {}, 0
    for phase in order:
        stop = cursor + phase_sizes[phase]
        phases[phase] = [cursor/RATE,stop/RATE]
        if phase in ("warm","tail"):
            far[cursor:cursor+len(signals[phase])] = signals[phase]
        if phase == "near":
            near[cursor:cursor+len(signals["near"])] = signals["near"]
        if phase in ("double","redouble"):
            far[cursor:cursor+len(signals["double_far"])] = signals["double_far"]
            near[cursor:cursor+len(signals["double_near"])] = signals["double_near"]
        cursor = stop
    echo = echo_room(far,"B",delay)
    if mode == "changed_path":
        boundary = round(phases["double"][0]*RATE)
        alternate = echo_room(far,"C",delay)
        echo[boundary:] = alternate[boundary:]
    if stress == "nonlinear":
        echo = np.tanh(3*echo)/3
    if ser_db is not None:
        dt = phases["double"]
        dn,de = selection(near,*dt),selection(echo,*dt)
        mask = v1.active_mask(dn)&v1.active_mask(de)
        near *= math.sqrt(v1.power(de[mask])/v1.power(dn[mask])) * 10**(ser_db/20)
    else:
        near *= 10**(near_db/20)
    noise = np.zeros(total)
    if stress == "noise":
        rng = np.random.default_rng(0xAECB2026)
        active = far+near
        noise = rng.standard_normal(total)
        noise *= math.sqrt(v1.power(active)/(10**(25/10)*v1.power(noise)))
        noise[round(phases["silence"][0]*RATE):] = 0
    if stress == "high_level":
        far,near,echo = [x*4 for x in (far,near,echo)]
    mic = near+echo+noise
    assert np.max(np.abs(mic)) <= 1 and np.max(np.abs(far)) <= 1, "fixture clipping"
    windows = {"near": [phases["near"][0]+1,phases["near"][1]],
               "double": phases.get("redouble",phases["double"]),
               "echo": [max(phases["tail"][0]+3,phases["tail"][1]-5),phases["tail"][1]],
               "silence": [phases["silence"][0]+1,phases["silence"][1]]}
    return mic,far,near,echo,windows,phases


def extended(out,espeak,report):
    signals,meta = heldout_sources(out,espeak)
    report["heldout_fixtures"] = meta
    report["rooms"] = ROOMS
    cases = [("standard",d,n,None,"none") for d in (0,40,80,120) for n in (0,-10,10)]
    cases += [("standard",40,0,ser,"none") for ser in (-10,10)]
    cases += [("cold",40,0,None,"none"),("changed_path",40,0,None,"none")]
    cases += [("standard",40,0,None,stress) for stress in ("noise","high_level","nonlinear")]
    for mode,delay,level,ser,stress in cases:
        mic,ref,n,echo,windows,phases = full_fixture(signals,delay,level,ser,mode,stress)
        h = fixed_h(n)
        y,elapsed = v1.process(out,mic,ref)
        r = judge(y,n,echo,h,windows)
        r.update(mode=mode,delay_ms=delay,near_scale_db=level,requested_ser_db=ser,stress=stress,
                 processing_seconds=elapsed,phases_seconds=phases,
                 microphone_peak=float(np.max(np.abs(mic))),reference_peak=float(np.max(np.abs(ref))))
        if mode in ("cold","changed_path"):
            start,end = phases["double"]
            r["transient_challenge"] = {"whole": double_metrics(y,n,echo,h,start,end),
                "first_1s": double_metrics(y,n,echo,h,start,start+1),
                "seconds_1_to_3": double_metrics(y,n,echo,h,start+1,start+3),
                "after_3s": double_metrics(y,n,echo,h,start+3,end)}
            # The whole transient window is diagnostic. Admission uses the
            # pre-frozen 3s convergence rule, plus near/echo/silence preservation.
            preserved = all(value for name,value in r["checks"].items() if not name.startswith("double_"))
            r["challenge_pass"] = preserved and r["transient_challenge"]["after_3s"]["pass"] and all(
                r["transient_challenge"][name]["checks"]["double_positive_projection"]
                for name in ("first_1s","seconds_1_to_3"))
        report["heldout"].append(r)
        print(f"HOLD mode={mode} delay={delay} near={level} ser={ser} stress={stress} pass={r['pass']} challenge={r.get('challenge_pass')} DTres={r['double']['metrics']['unscaled_h_residual_reduction_db']:.3f}",flush=True)
        save(out,report)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out",type=Path,default=ROOT/"outputs/022-meeting/aec-v2")
    parser.add_argument("--espeak",type=Path,default=Path("C:/Program Files/eSpeak NG/espeak-ng.exe"))
    parser.add_argument("--reuse-native",action="store_true",help="Only reuse when stored build source hashes exactly match")
    parser.add_argument("--extended",action="store_true")
    parser.add_argument("--reference-only",action="store_true")
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True,exist_ok=True)
    frozen = {**PROTOCOL,"frozen_utc": datetime.now(timezone.utc).isoformat()}
    (out/"protocol-frozen.json").write_text(json.dumps(frozen,indent=2)+"\n",encoding="utf-8")
    reference_test = h_self_test()
    (out/"independent-h-selftest.json").write_text(json.dumps(reference_test,indent=2)+"\n",encoding="utf-8")
    print("Independent analytic H self-test PASS before any native output",flush=True)
    if args.reference_only:
        return
    sources = provenance()
    native_sources = {k:v for k,v in sources.items() if "/native/" in k}
    build_info_path = out/"native-build-provenance.json"
    if args.reuse_native:
        info = json.loads(build_info_path.read_text(encoding="utf-8"))
        assert info["native_source_sha256"] == native_sources, "native source mismatch; rebuild required"
        assert info["native_lib_sha256"] == sha(out/"meeting_speexdsp.lib"), "native binary mismatch"
        assert info["builder_sha256"] == sha(ROOT/"scripts/check-meeting-aec.py"), "builder changed; rebuild required"
    else:
        v1.native_build(out)
        info = {"native_source_sha256": native_sources,"native_lib_sha256": sha(out/"meeting_speexdsp.lib"),
                "builder_sha256": sha(ROOT/"scripts/check-meeting-aec.py"),
                "flags": "/O2 /MD FLOATING_POINT USE_SMALLFT EXPORT= inline=__inline",
                "built_utc": datetime.now(timezone.utc).isoformat()}
        build_info_path.write_text(json.dumps(info,indent=2)+"\n",encoding="utf-8")
    v1.rust_build(out)
    unit = subprocess.run([str(out/"aec-tests.exe"),"--nocapture"],capture_output=True,text=True)
    (out/"unit-tests.log").write_text(unit.stdout+unit.stderr,encoding="utf-8")
    print(unit.stdout+unit.stderr,flush=True)
    report = {"pass": False,"complete": False,"kind": "synthetic_offline_no_playback",
              "protocol": frozen,"independent_h_selftest": reference_test,"unit_tests_pass": unit.returncode == 0,
              "source_sha256": sources,"native_provenance": info,"probe_sha256": sha(out/"aec-probe.exe"),
              "baseline": [],"heldout": [],"old_v1_report_sha256": sha(ROOT/"outputs/022-meeting/aec/aec-report.json")}
    baseline(out,args.espeak,report)
    report["baseline_pass"] = unit.returncode == 0 and all(r["pass"] for r in report["baseline"])
    if args.extended:
        extended(out,args.espeak,report)
    report["heldout_standard_pass"] = bool(report["heldout"]) and all(r["pass"] for r in report["heldout"] if r["mode"] == "standard" and r["stress"] == "none")
    report["challenge_pass"] = bool(report["heldout"]) and all(r.get("challenge_pass",r["pass"]) for r in report["heldout"])
    report["pass"] = report["baseline_pass"] and (not args.extended or report["heldout_standard_pass"] and report["challenge_pass"])
    report["complete"] = True
    report["provenance_stable_through_run"] = sources == provenance()
    report["pass"] = report["pass"] and report["provenance_stable_through_run"]
    save(out,report)
    print(f"REPORT {out/'aec-v2-report.json'} pass={report['pass']}",flush=True)
    raise SystemExit(0 if report["pass"] else 1)


if __name__ == "__main__":
    main()
