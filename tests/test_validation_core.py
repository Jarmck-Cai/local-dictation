import json
import os
import time
from datetime import datetime, timedelta, timezone

import pytest

from validation.core import mixed_error_rate, percentile, prune_recordings, load_local_env
from validation.providers import build_claude_request, clean_text, validate_response


def test_mixed_accuracy_ignores_case_and_punctuation_but_counts_wrong_english():
    assert mixed_error_rate("请用 Python 跑 API。", "请用 python 跑 API") == 0
    assert mixed_error_rate("请用 Python", "请用 Java") == pytest.approx(1 / 3)


def test_percentile_uses_interpolation_and_rejects_empty_measurements():
    assert percentile([1, 2, 3], 0.5) == 2
    assert percentile([1, 2, 3], 0.95) == pytest.approx(2.9)
    assert percentile([], 0.95) is None


def test_recording_retention_deletes_only_expired_wavs_inside_its_root(tmp_path):
    now = datetime(2026, 10, 26, tzinfo=timezone.utc)
    expired = tmp_path / "expired.wav"
    recent = tmp_path / "recent.wav"
    notes = tmp_path / "keep.json"
    for file in (expired, recent, notes):
        file.write_bytes(b"example")
    old = (now - timedelta(days=7, seconds=1)).timestamp()
    os.utime(expired, (old, old))
    os.utime(notes, (old, old))
    os.utime(recent, (now.timestamp(), now.timestamp()))
    assert prune_recordings(tmp_path, now.timestamp()) == ["expired.wav"]
    assert recent.exists() and notes.exists()


def test_local_env_does_not_execute_or_override_existing_credentials(tmp_path, monkeypatch):
    env = tmp_path / ".env.local"
    env.write_text("ANTHROPIC_API_KEY=new\nCLAUDE_MODEL=claude-haiku-5-5\nIGNORED=$(whoami)\n", encoding="utf-8")
    monkeypatch.setenv("ANTHROPIC_API_KEY", "existing")
    monkeypatch.delenv("CLAUDE_MODEL", raising=False)
    load_local_env(env)
    assert os.environ["ANTHROPIC_API_KEY"] == "existing"
    assert os.environ["CLAUDE_MODEL"] == "claude-haiku-5-5"
    assert "IGNORED" not in os.environ


def test_claude_request_contains_only_text_and_disables_thinking():
    body = build_claude_request("请用 Python 跑一下 API", "claude-haiku-5-5")
    assert body["thinking"] == {"type": "disabled"}
    assert body["output_config"] == {"effort": "low"}
    assert "temperature" not in body
    assert body["messages"] == [{"role": "user", "content": "请用 Python 跑一下 API"}]
    assert "audio" not in json.dumps(body)


@pytest.mark.parametrize("payload", [
    {"stop_reason": "max_tokens", "content": [{"type": "text", "text": "truncated"}]},
    {"stop_reason": "end_turn", "content": [{"type": "thinking", "thinking": "hidden"}]},
    {"stop_reason": "refusal", "content": [{"type": "text", "text": "no"}]},
])
def test_incomplete_empty_or_refused_results_never_replace_raw_transcript(payload):
    with pytest.raises(ValueError):
        validate_response(payload, "claude", "原文")


def test_changed_digits_are_rejected_but_unchanged_digits_are_accepted():
    body = {"stop_reason": "end_turn", "content": [{"type": "text", "text": "不是25，是26。"}]}
    assert validate_response(body, "claude", "不是25是26") == "不是25，是26。"
    with pytest.raises(ValueError, match="digits_changed"):
        validate_response(body, "claude", "不是25是27")


def test_missing_key_falls_back_without_network(monkeypatch):
    monkeypatch.delenv("ANTHROPIC_API_KEY", raising=False)
    result = clean_text("保留我的原始转录", "claude")
    assert result["text"] == "保留我的原始转录"
    assert result["status"] == "fallback"
    assert result["error"] == "missing_anthropic_api_key"


def test_local_mode_rejects_remote_hosts_without_sending_text(monkeypatch):
    monkeypatch.setenv("LOCAL_LLM_BASE_URL", "https://example.com/v1")
    result = clean_text("不能偷偷上传", "local")
    assert result["status"] == "fallback"
    assert result["error"] == "local_endpoint_must_be_loopback"


def test_off_mode_never_invokes_an_api():
    assert clean_text("直接转录", "off")["status"] == "off"


def test_timeout_returns_raw_once_and_ignores_a_late_answer(monkeypatch):
    monkeypatch.setenv("ANTHROPIC_API_KEY", "test-not-a-real-key")

    def late_transport(*args):
        time.sleep(0.05)
        return {"stop_reason": "end_turn", "content": [{"type": "text", "text": "迟到的整理"}]}

    result = clean_text("原文", "claude", timeout=0.005, transport=late_transport)
    assert result["error"] == "cleanup_timeout"
    time.sleep(0.07)
    assert result["text"] == "原文" and result["status"] == "fallback"


def test_text_cleanup_does_not_use_thinking_blocks_as_final_text(monkeypatch):
    monkeypatch.setenv("ANTHROPIC_API_KEY", "test-not-a-real-key")
    result = clean_text("原文", "claude", transport=lambda *args: {
        "stop_reason": "end_turn", "content": [{"type": "thinking", "thinking": "内部内容"},
        {"type": "text", "text": "原文。"}], "usage": {"input_tokens": 12, "output_tokens": 3}})
    assert result["text"] == "原文。" and result["status"] == "ok"
    assert result["usage"]["input_tokens"] == 12


def test_truncated_response_preserves_usage_for_billed_failed_requests(monkeypatch):
    monkeypatch.setenv("ANTHROPIC_API_KEY", "test-not-a-real-key")
    result = clean_text("原文", "claude", transport=lambda *args: {
        "stop_reason": "max_tokens", "content": [{"type": "text", "text": "半句"}],
        "usage": {"input_tokens": 30, "output_tokens": 5}})
    assert result["text"] == "原文" and result["status"] == "fallback"
    assert result["usage"]["output_tokens"] == 5


def test_blank_environment_key_can_be_filled_after_tool_start(tmp_path, monkeypatch):
    monkeypatch.setenv("ANTHROPIC_API_KEY", "")
    env = tmp_path / ".env.local"
    env.write_text("ANTHROPIC_API_KEY=test-placeholder-not-real\n", encoding="utf-8")
    load_local_env(env)
    assert os.environ["ANTHROPIC_API_KEY"] == "test-placeholder-not-real"


@pytest.mark.parametrize("raw,changed", [
    ("threshold 设为 -12，不要设为12。", "threshold 设为12，不要设为12。"),
    ("费率是12%，不是12元。", "费率是12，不是12元。"),
    ("温度为−3.5度。", "温度为3.5度。"),
])
def test_numeric_guard_preserves_sign_and_percent(raw, changed):
    payload = {"stop_reason": "end_turn", "content": [{"type": "text", "text": changed}]}
    with pytest.raises(ValueError, match="digits_changed"):
        validate_response(payload, "claude", raw)
