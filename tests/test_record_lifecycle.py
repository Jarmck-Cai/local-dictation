"""Record lifecycle with controlled OS/audio boundaries; never use a microphone."""
import json
import sys
from types import SimpleNamespace
import wave

import numpy as np
import pytest

from validation import run


def result_fixture(cleanup_peak=640):
    return {"raw_text": "测试", "final_text": "测试。", "audio_seconds": 0.02,
            "asr": {"wall_seconds": 0.1, "rss_peak_mib": 380, "cpu_machine_percent": 1},
            "cleanup_resources": {"rss_peak_mib": cleanup_peak},
            "cleanup": {"elapsed_seconds": 0.2, "error": None},
            "processing_seconds": 0.3, "release_to_preview_seconds": 0.3,
            "raw_mixed_error_rate": None}


def test_terminal_includes_cleanup_memory_peak(capsys):
    # A summary that reads only ASR would under-report a larger cleanup process peak.
    result = result_fixture()
    run.print_result(result)
    assert "640 MiB" in capsys.readouterr().out


def test_saved_summary_includes_cleanup_memory_peak(tmp_path, monkeypatch):
    result = result_fixture()
    result["case"] = "recording"
    monkeypatch.setattr(run, "gpu_snapshot", lambda: None)
    destination = tmp_path / "new-session.json"
    run.save_session({"runs": [result], "report_path": str(destination)})
    assert json.loads(destination.read_text(encoding="utf-8"))["summary"]["recording"]["peak_rss_mib"] == 640


def test_old_result_without_cleanup_resources_still_prints_and_saves(tmp_path, monkeypatch, capsys):
    result = result_fixture()
    result.pop("cleanup_resources")
    result["case"] = "legacy"
    run.print_result(result)
    assert "380 MiB" in capsys.readouterr().out
    monkeypatch.setattr(run, "gpu_snapshot", lambda: None)
    info = {"runs": [result], "report_path": str(tmp_path / "summary.json")}
    run.save_session(info)
    assert info["summary"]["legacy"]["peak_rss_mib"] == 380


class RecordingHarness:
    """Replace external endpoint, OS keys and input stream, keeping record/WAV/JSON real."""

    def __init__(self, tmp_path, monkeypatch, *, outcome="release", mute=True, mute_error=False,
                 restore_error=False, original_mute=False):
        from validation.audio import PlaybackMute

        self.clock = 10.0
        self.muted = original_mute
        self.original_mute = original_mute
        self.events = []
        self.active = False
        self.done = False
        self.cancelled = False
        self.inner_checks = 0
        self.outcome = outcome
        self.released_at = None
        self.info = {"runs": [], "report_path": str(tmp_path / "session.json")}
        self.args = SimpleNamespace(mode="off", device=None, max_seconds=0.15 if outcome == "limit" else 2,
                                    timeout=1, mute_playback=mute)
        harness = self

        class Endpoint:
            def get_mute(self):
                return harness.muted

            def set_mute(self, value):
                if value and mute_error:
                    raise OSError("mute unavailable")
                if harness.active is False and "stream_close" in harness.events and restore_error:
                    raise OSError("restore unavailable")
                harness.muted = value
                harness.events.append("mute" if value else "restore")
                harness.clock += 0.3

            def close(self):
                harness.events.append("endpoint_close")

        class Stream:
            device = 7  # Resolved device differs from args.device's default selector.

            def __init__(self, **kwargs):
                harness.events.append("stream_construct")
                harness.muted_on_open = harness.muted
                if outcome == "open_error":
                    raise OSError("input stream unavailable")
                self.receive = kwargs["callback"]

            def __enter__(self):
                harness.active = True
                harness.events.append("stream_open")
                self.receive(np.array([0, 400, -200, 0], dtype="<i2").tobytes(), 4, None, "")
                return self

            def __exit__(self, *exc):
                harness.muted_on_close = harness.muted
                harness.events.append("stream_close")
                harness.clock += 0.2
                harness.active = False

        def pressed(key):
            if key == 0x1B:
                if outcome == "cancel" and harness.active:
                    harness.cancelled = True
                return 0x8000 if harness.done or harness.cancelled else 0
            if not harness.active:
                return 0x8000
            harness.inner_checks += 1
            return 0x8000 if outcome == "limit" or harness.inner_checks <= 1 else 0

        def sleep(seconds):
            if harness.active and outcome == "interrupt":
                raise KeyboardInterrupt()
            if harness.active and outcome == "record_error":
                raise RuntimeError("record loop failed")
            harness.clock += 0.1

        def analyze(engine, samples, rate, mode, timeout, *, released_at):
            # Fail if restoration moves outside recording scope or after ASR/cleanup.
            assert harness.muted is original_mute or restore_error
            assert harness.events[-1] == "endpoint_close" or not mute
            harness.released_at = released_at
            harness.analyze_at = harness.clock
            harness.done = True
            assert samples.tolist() == [0, 400 / 32768, -200 / 32768, 0]
            assert rate == 16000
            return result_fixture()

        monkeypatch.setattr(run, "PlaybackMute", lambda enabled=True: PlaybackMute(
            enabled=enabled, endpoint_factory=lambda: Endpoint()), raising=False)
        monkeypatch.setattr(run, "ROOT", tmp_path)
        monkeypatch.setattr(run, "RECORDINGS", tmp_path / "recordings")
        monkeypatch.setattr(run, "prune_recordings", lambda: [])
        monkeypatch.setattr(run, "load_local_env", lambda: None)
        monkeypatch.setattr(run, "new_session", lambda args: (object(), harness.info))
        monkeypatch.setattr(run, "analyze", analyze)
        monkeypatch.setattr(run, "gpu_snapshot", lambda: None)
        monkeypatch.setattr(run, "time", SimpleNamespace(monotonic=lambda: harness.clock,
                            perf_counter=lambda: harness.clock, sleep=sleep))
        monkeypatch.setattr(run.ctypes, "windll", SimpleNamespace(user32=SimpleNamespace(GetAsyncKeyState=pressed)))
        monkeypatch.setitem(sys.modules, "sounddevice", SimpleNamespace(
            RawInputStream=Stream,
            query_devices=self.query_device,
            query_hostapis=lambda index: {"name": "Windows WASAPI"}))

    def query_device(self, index):
        assert index == 7
        if self.outcome == "metadata_error":
            raise OSError("device disconnected while querying metadata")
        return {"name": "Actual USB microphone", "hostapi": 2}


def test_record_mutes_only_stream_and_restores_before_processing(tmp_path, monkeypatch):
    harness = RecordingHarness(tmp_path, monkeypatch)
    run.record(harness.args)
    assert harness.muted_on_open is True and harness.muted_on_close is True
    assert harness.muted is False
    assert harness.released_at < harness.analyze_at - 0.4
    saved = json.loads((tmp_path / "session.json").read_text(encoding="utf-8"))["runs"][0]
    assert saved["playback_mute"]["restored"] is True
    assert saved["input_device"] == {"index": 7, "name": "Actual USB microphone",
                                      "hostapi": {"index": 2, "name": "Windows WASAPI"}}
    with wave.open(saved["recording"], "rb") as recording:
        assert recording.getnframes() == 4
        assert recording.getframerate() == 16000


@pytest.mark.parametrize("outcome,error", [("open_error", OSError), ("interrupt", KeyboardInterrupt),
                                           ("record_error", RuntimeError)])
def test_record_restores_and_propagates_stream_or_loop_failure(tmp_path, monkeypatch, outcome, error):
    harness = RecordingHarness(tmp_path, monkeypatch, outcome=outcome)
    with pytest.raises(error):
        run.record(harness.args)
    assert harness.muted is False
    assert harness.events[-1] == "endpoint_close"
    assert not harness.info["runs"]
    assert harness.info["recording_attempts"][0]["playback_mute"]["restored"] is True
    assert not list((tmp_path / "recordings").glob("*.wav"))
    assert (tmp_path / "session.json").exists()


def test_escape_cancels_recording_and_restores(tmp_path, monkeypatch):
    harness = RecordingHarness(tmp_path, monkeypatch, outcome="cancel")
    run.record(harness.args)
    assert harness.muted is False
    assert harness.events[-1] == "endpoint_close"
    assert not harness.info["runs"]
    assert harness.info["recording_attempts"][0]["playback_mute"]["restored"] is True
    assert not list((tmp_path / "recordings").glob("*.wav"))


def test_duration_limit_restores_and_saves_automatic_stop(tmp_path, monkeypatch):
    harness = RecordingHarness(tmp_path, monkeypatch, outcome="limit")
    run.record(harness.args)
    assert harness.muted is False
    assert harness.info["runs"][0]["stop_reason"] == "duration_limit"
    assert harness.info["runs"][0]["release_to_preview_seconds"] is None


def test_disable_mute_preserves_playback_and_reports_disabled(tmp_path, monkeypatch):
    harness = RecordingHarness(tmp_path, monkeypatch, mute=False)
    run.record(harness.args)
    assert harness.muted_on_open is False and harness.muted_on_close is False
    assert harness.info["runs"][0]["playback_mute"]["status"] == "disabled"


def test_record_preserves_an_already_muted_endpoint(tmp_path, monkeypatch):
    harness = RecordingHarness(tmp_path, monkeypatch, original_mute=True)
    run.record(harness.args)
    assert harness.muted is True
    assert harness.info["runs"][0]["playback_mute"]["original_mute"] is True


def test_metadata_query_failure_does_not_lose_recording_or_restoration(tmp_path, monkeypatch):
    harness = RecordingHarness(tmp_path, monkeypatch, outcome="metadata_error")
    run.record(harness.args)
    result = harness.info["runs"][0]
    assert result["playback_mute"]["restored"] is True
    assert result["input_device"]["index"] == 7
    assert result["input_device"]["metadata_error"]


@pytest.mark.parametrize("failure", ["mute", "restore"])
def test_audio_guard_failure_is_visible_and_recorded(tmp_path, monkeypatch, capsys, failure):
    harness = RecordingHarness(tmp_path, monkeypatch, mute_error=failure == "mute", restore_error=failure == "restore")
    run.record(harness.args)
    report = harness.info["runs"][0]["playback_mute"]
    assert report["errors"]
    output = capsys.readouterr().out
    if failure == "restore":
        assert "手动" in output and "恢复" in output
        assert report["status"] == "restore_failed"
    else:
        assert "静音失败" in output
        assert report["status"] == "failed"


def test_restore_failure_is_reported_even_when_recording_raises(tmp_path, monkeypatch, capsys):
    harness = RecordingHarness(tmp_path, monkeypatch, outcome="record_error", restore_error=True)
    with pytest.raises(RuntimeError, match="record loop failed"):
        run.record(harness.args)
    output = capsys.readouterr().out
    assert "手动" in output and "恢复" in output
    saved = json.loads((tmp_path / "session.json").read_text(encoding="utf-8"))
    assert saved["recording_attempts"][0]["playback_mute"]["status"] == "restore_failed"
    assert saved["runs"] == []


@pytest.mark.parametrize("flags,expected", [([], True), (["--no-mute-playback"], False)])
def test_record_command_parses_mute_default_and_opt_out(monkeypatch, flags, expected):
    observed = []
    monkeypatch.setattr(sys, "argv", ["validation.run", "record", *flags])
    monkeypatch.setattr(run, "record", lambda args: observed.append(args.mute_playback))
    run.main()
    assert observed == [expected]


@pytest.mark.parametrize("command", ["bench", "devices", "cleanup"])
def test_non_record_commands_never_open_playback_endpoint(tmp_path, monkeypatch, command):
    def forbidden(*args, **kwargs):
        raise AssertionError("non-record command attempted playback control")

    monkeypatch.setattr(run, "PlaybackMute", forbidden)
    monkeypatch.setattr(run, "gpu_snapshot", lambda: None)
    monkeypatch.setattr(run, "prune_recordings", lambda: [])
    monkeypatch.setitem(sys.modules, "sounddevice", SimpleNamespace(query_devices=lambda: []))
    argv = ["validation.run", command, "--mode", "off"]
    if command == "bench":
        path = tmp_path / "fixture.wav"
        with wave.open(str(path), "wb") as output:
            output.setnchannels(1)
            output.setsampwidth(2)
            output.setframerate(16000)
            output.writeframes(b"\x00\x00" * 320)
        info = {"runs": [], "report_path": str(tmp_path / "bench.json")}
        monkeypatch.setattr(run, "new_session", lambda args: (object(), info))
        monkeypatch.setattr(run, "analyze", lambda *args: result_fixture())
        argv.extend(["--wav", str(path), "--repeat", "1"])
    monkeypatch.setattr(sys, "argv", argv)
    run.main()
    if command == "bench":
        assert json.loads((tmp_path / "bench.json").read_text(encoding="utf-8"))["runs"]
