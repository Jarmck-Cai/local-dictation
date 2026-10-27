"""Run with python -m validation.run --help."""
from __future__ import annotations

import argparse
from collections import defaultdict
import ctypes
from datetime import datetime, timezone
import importlib.metadata
import json
import os
from pathlib import Path
import platform
import time
import uuid
import wave

import numpy as np

from .asr import SenseVoice, read_wav
from .audio import PlaybackMute
from .core import MODEL_DIR, RECORDINGS, ROOT, load_local_env, mixed_error_rate, percentile, prune_recordings
from .measure import Meter, gpu_snapshot
from .providers import clean_text

ZH_REFERENCE = "开放时间早上9点至下午5点。"
EN_REFERENCE = "The tribal chieftain called for the boy and presented him with 50 pieces of gold."


def analyze(engine, samples, rate, mode, timeout, reference=None, released_at=None):
    started = time.perf_counter() if released_at is None else released_at
    with Meter() as transcription:
        raw = engine.decode(samples, rate)
    with Meter() as cleanup:
        polished = clean_text(raw, mode, timeout)
    ready = time.perf_counter()
    return {"audio_seconds": len(samples) / rate, "segments": engine.last_segments,
            "raw_text": raw, "final_text": polished["text"],
            "asr": transcription.result, "cleanup_resources": cleanup.result, "cleanup": polished,
            "processing_seconds": ready - started,
            "release_to_preview_seconds": ready - started if released_at is not None else None,
            "real_time_factor": transcription.result["wall_seconds"] / max(len(samples) / rate, 1e-9),
            "reference": reference,
            "raw_mixed_error_rate": mixed_error_rate(reference, raw) if reference else None,
            "final_mixed_error_rate": mixed_error_rate(reference, polished["text"]) if reference else None}


def peak_rss_mib(result):
    return max(result["asr"]["rss_peak_mib"],
               result.get("cleanup_resources", {}).get("rss_peak_mib", 0))


def print_result(result):
    print(f"\n原始转录：{result['raw_text'][:600]}")
    print(f"最终文字：{result['final_text'][:600]}")
    print(f"录音 {result['audio_seconds']:.2f}s | 识别 {result['asr']['wall_seconds']:.3f}s | "
          f"整理 {result['cleanup']['elapsed_seconds']:.3f}s | "
          f"总处理 {result['processing_seconds']:.3f}s | "
          f"进程峰值内存 {peak_rss_mib(result):.0f} MiB | "
          f"识别阶段 CPU {result['asr']['cpu_machine_percent']:.1f}%")
    if result["cleanup"]["error"]:
        print("整理回退：" + result["cleanup"]["error"])
    if result["raw_mixed_error_rate"] is not None:
        print(f"原始混合 token 错误率：{100 * result['raw_mixed_error_rate']:.1f}%")
    print(flush=True)


def new_session(args):
    load_local_env()
    with Meter() as loading:
        engine = SenseVoice(args.threads, segmentation=args.segmentation)
    # Warm the computation path, separately from model construction and measured speech.
    with Meter() as warming:
        engine.decode(np.zeros(8000, dtype=np.float32), 16000)
    info = {"created_at": datetime.now(timezone.utc).isoformat(), "kind": args.command,
            "platform": platform.platform(), "backend": "sherpa-onnx CPU; not Handy runtime",
            "model": MODEL_DIR.name, "threads": args.threads, "segmentation": args.segmentation,
            "cleanup_mode": args.mode,
            "packages": {name: importlib.metadata.version(name) for name in
                         ("sherpa-onnx", "numpy", "sounddevice", "psutil")},
            "cold_model_load": loading.result, "silent_warmup": warming.result,
            "gpu_device_before": gpu_snapshot(), "runs": [],
            "scope": "Final text preview only; no clipboard/paste/target-window input is measured."}
    print(f"模型加载 {loading.result['wall_seconds']:.3f}s；预热 {warming.result['wall_seconds']:.3f}s。", flush=True)
    return engine, info


def save_session(info):
    groups = defaultdict(list)
    for run in info["runs"]:
        groups[run["case"]].append(run)
    info["summary"] = {name: {
        "count": len(runs), "low_sample_count": len(runs) < 20,
        "asr_p50_seconds": percentile([r["asr"]["wall_seconds"] for r in runs], 0.5),
        "asr_p95_seconds": percentile([r["asr"]["wall_seconds"] for r in runs], 0.95),
        "processing_p50_seconds": percentile([r["processing_seconds"] for r in runs], 0.5),
        "processing_p95_seconds": percentile([r["processing_seconds"] for r in runs], 0.95),
        "peak_rss_mib": max(peak_rss_mib(r) for r in runs),
    } for name, runs in groups.items()}
    info["gpu_device_after"] = gpu_snapshot()
    if "report_path" not in info:
        info["report_path"] = str(ROOT / "outputs" / "validation" / (
            datetime.now().strftime("%Y%m%d-%H%M%S") + "-" + uuid.uuid4().hex[:6] + ".json"))
    destination = Path(info["report_path"])
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_text(json.dumps(info, ensure_ascii=False, indent=2), encoding="utf-8")
    print(f"详细记录：{destination}", flush=True)
    return destination


def bench(args):
    engine, info = new_session(args)
    samples = MODEL_DIR / "test_wavs"
    cases = []
    if args.wav:
        reference = args.reference.read_text(encoding="utf-8-sig") if args.reference else None
        for path in args.wav:
            audio, rate = read_wav(path)
            cases.append((path.name, audio, rate, reference, "provided_wav"))
    else:
        zh, rate = read_wav(samples / "zh.wav")
        en, en_rate = read_wav(samples / "en.wav")
        if rate != en_rate:
            raise ValueError("Official sample rates differ")
        combined = np.concatenate([zh, np.zeros(int(rate * 0.35), dtype=np.float32), en])
        cases.extend([
            ("official_zh", zh, rate, ZH_REFERENCE, "official_human_sample"),
            ("official_en", en, rate, EN_REFERENCE, "official_human_sample"),
            ("zh_en_splice", combined, rate, ZH_REFERENCE + " " + EN_REFERENCE,
             "concatenated_separate_speakers_not_natural_code_switching"),
        ])
        for seconds in args.stress_seconds:
            # Repetition is a resource/length stress test, not an accuracy evaluation.
            repeated = np.tile(combined, int(np.ceil(seconds * rate / len(combined))))[:int(seconds * rate)]
            cases.append((f"repeated_{seconds}s", repeated, rate, None, "artificial_repetition_stress_only"))
    try:
        for name, audio, rate, reference, provenance in cases:
            for iteration in range(args.repeat):
                print(f"测试 {name} 第 {iteration + 1}/{args.repeat} 次", flush=True)
                result = analyze(engine, audio, rate, args.mode, args.timeout, reference)
                result.update(case=name, iteration=iteration + 1, provenance=provenance)
                info["runs"].append(result)
                print_result(result)
    finally:
        save_session(info)


def record(args):
    if os.name != "nt":
        raise ValueError("F8 录音入口仅用于 Windows")
    import sounddevice as sd
    RECORDINGS.mkdir(parents=True, exist_ok=True)
    if not RECORDINGS.resolve().is_relative_to(ROOT.resolve()):
        raise ValueError("录音缓存目录必须位于项目内")
    prune_recordings()
    engine, info = new_session(args)
    if args.mode == "claude" and not os.environ.get("ANTHROPIC_API_KEY"):
        print("未配置 Claude 密钥：可以先录音，整理会回退到原文。")
    pressed = lambda key: bool(ctypes.windll.user32.GetAsyncKeyState(key) & 0x8000)
    last_cleanup = time.monotonic()
    print("\n按住 F8 说话，松开后查看结果；Esc 退出。仅显示结果，不自动粘贴。", flush=True)
    try:
        while not pressed(0x1B):
            if time.monotonic() - last_cleanup > 60:
                prune_recordings()
                last_cleanup = time.monotonic()
            if not pressed(0x77):
                time.sleep(0.01)
                continue
            blocks, overflows = [], []

            def receive(data, frames, timing, status):
                blocks.append(bytes(data))
                if status:
                    overflows.append(str(status))

            playback = PlaybackMute(enabled=args.mute_playback)
            attempt = {"playback_mute": playback.report, "input_device": None,
                       "status": "recording_failed"}
            info.setdefault("recording_attempts", []).append(attempt)
            try:
                with playback:
                    if playback.report.get("errors"):
                        print("播放静音失败：仍继续录音，请注意背景播放可能混入。", flush=True)
                    with sd.RawInputStream(samplerate=16000, channels=1, dtype="int16", device=args.device,
                                           callback=receive, blocksize=320) as stream:
                        # stream.device is the resolved input, unlike args.device's default selector.
                        input_device = {"index": None, "name": None, "hostapi": None}
                        attempt["input_device"] = input_device
                        try:
                            input_device["index"] = int(stream.device)
                            device = sd.query_devices(input_device["index"])
                            input_device["name"] = device["name"]
                            hostapi = int(device["hostapi"])
                            input_device["hostapi"] = {"index": hostapi,
                                                       "name": sd.query_hostapis(hostapi)["name"]}
                        except Exception as error:
                            input_device["metadata_error"] = f"{type(error).__name__}: {error}"
                        began = time.perf_counter()
                        print("录音中……", flush=True)
                        while pressed(0x77) and not pressed(0x1B) and time.perf_counter() - began < args.max_seconds:
                            time.sleep(0.005)
                        released = time.perf_counter()
                        automatic_stop = released - began >= args.max_seconds
            finally:
                attempt["playback_mute"] = playback.report
                if playback.report.get("status") == "restore_failed":
                    print("播放静音恢复失败：请在 Windows 音量设置中手动恢复原播放设备的静音状态。", flush=True)
                elif any(error.get("stage") == "close" for error in playback.report.get("errors", [])):
                    print("播放设备资源释放失败，详细错误已记录。", flush=True)
            # Close/drain, playback restoration, PCM assembly, persistence and inference
            # are inside release-to-preview; restoration finishes before processing.
            pcm = b"".join(blocks)
            if not pcm or pressed(0x1B):
                attempt["status"] = "cancelled" if pcm else "empty"
                continue
            attempt["status"] = "captured"
            name = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ") + "-" + uuid.uuid4().hex[:8] + ".wav"
            path = RECORDINGS / name
            with wave.open(str(path), "wb") as output:
                output.setnchannels(1)
                output.setsampwidth(2)
                output.setframerate(16000)
                output.writeframes(pcm)
            audio = np.frombuffer(pcm, dtype="<i2").astype(np.float32) / 32768
            load_local_env()
            result = analyze(engine, audio, 16000, args.mode, args.timeout, released_at=released)
            if automatic_stop:
                result["release_to_preview_seconds"] = None
            result.update(case=name, provenance="user_microphone", recording=str(path), audio_status=overflows,
                          input_device=input_device, playback_mute=playback.report,
                          stop_reason="duration_limit" if automatic_stop else "key_release")
            info["runs"].append(result)
            print_result(result)
            save_session(info)
            print("继续按住 F8 录下一条；Esc 退出。", flush=True)
            while pressed(0x77) and not pressed(0x1B):
                time.sleep(0.01)
    finally:
        save_session(info)


def main():
    parser = argparse.ArgumentParser(description="本地语音输入最小验证：F8 录音或 WAV 基准测试")
    parser.add_argument("command", choices=["bench", "record", "devices", "cleanup"])
    parser.add_argument("--mode", choices=["claude", "local", "off"], default="claude")
    parser.add_argument("--threads", type=int, default=4)
    parser.add_argument("--segmentation", choices=["vad", "whole"], default="vad")
    parser.add_argument("--timeout", type=float, default=12, help="整理等待上限，秒；长语音可提高")
    parser.add_argument("--repeat", type=int, default=3)
    parser.add_argument("--stress-seconds", type=int, nargs="*", default=[])
    parser.add_argument("--wav", type=Path, nargs="*")
    parser.add_argument("--reference", type=Path)
    parser.add_argument("--device", type=int)
    parser.add_argument("--max-seconds", type=int, default=180)
    parser.add_argument("--mute-playback", action=argparse.BooleanOptionalAction, default=True,
                        help="录音期间静音默认播放设备，结束后恢复原状态（默认开启）")
    args = parser.parse_args()
    if min(args.threads, args.timeout, args.repeat, args.max_seconds) <= 0:
        parser.error("线程数、超时、重复次数与最长录音时长必须大于零")
    if args.command == "devices":
        import sounddevice as sd
        print(sd.query_devices())
    elif args.command == "cleanup":
        print(json.dumps(prune_recordings(), ensure_ascii=False))
    else:
        {"bench": bench, "record": record}[args.command](args)


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        print("\n验证已停止。")
