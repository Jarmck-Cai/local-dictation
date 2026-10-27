import pytest

from validation.providers import clean_text, validate_response


RAW = "好的，我们再来尝试一段更加长的录音。"
PREFACE = "好的，我会按照保守整理的方式处理这段转录内容：\n\n好的，我们再尝试一段更长的录音。"


def _response(mode, output):
    if mode == "claude":
        return {
            "stop_reason": "end_turn",
            "content": [{"type": "text", "text": output}],
            "usage": {"input_tokens": 17, "output_tokens": 12},
            "model": "claude-test-model",
        }
    return {
        "choices": [{"finish_reason": "stop", "message": {"content": output}}],
        "usage": {"prompt_tokens": 17, "completion_tokens": 12},
        "model": "local-test-model",
    }


@pytest.mark.parametrize("mode", ["claude", "local"])
def test_new_transcription_preface_falls_back_and_keeps_billing_metadata(mode, monkeypatch):
    monkeypatch.setenv("ANTHROPIC_API_KEY", "test-placeholder-not-a-real-key")
    calls = []

    def transport(*args):
        calls.append(args)
        return _response(mode, PREFACE)

    result = clean_text(RAW, mode, transport=transport)

    assert result["text"] == RAW
    assert result["status"] == "fallback"
    assert result["error"] == "unexpected_preface"
    assert result["usage"] == (
        {"input_tokens": 17, "output_tokens": 12}
        if mode == "claude"
        else {"prompt_tokens": 17, "completion_tokens": 12}
    )
    assert result["model"] == ("claude-test-model" if mode == "claude" else "local-test-model")
    assert len(calls) == 1


@pytest.mark.parametrize("mode,output", [
    ("claude", "整理后的文字如下：\n\n转录中的原话是：‘请说“整理后的文字如下”。’"),
    ("local", "Here is the cleaned-up transcript:\n\nThe clip says: “Here is the cleaned-up transcript.”"),
])
def test_preface_like_words_already_in_raw_are_not_rejected(mode, output, monkeypatch):
    monkeypatch.setenv("ANTHROPIC_API_KEY", "test-placeholder-not-a-real-key")
    raw = output
    result = clean_text(raw, mode, transport=lambda *args: _response(mode, output))

    assert result["status"] == "ok"
    assert result["text"] == output


@pytest.mark.parametrize("mode,raw,output", [
    ("claude", "好的，我们再试一段。", "好的，我们再试一段。"),
    ("local", "Okay, let's try another recording.", "Okay, let's try another recording."),
])
def test_normal_spoken_opening_is_not_mistaken_for_an_intro(mode, raw, output, monkeypatch):
    monkeypatch.setenv("ANTHROPIC_API_KEY", "test-placeholder-not-a-real-key")
    result = clean_text(raw, mode, transport=lambda *args: _response(mode, output))

    assert result["status"] == "ok"
    assert result["text"] == output


@pytest.mark.parametrize("mode,output", [
    ("claude", chr(10).join(("以下是整理后的转录内容：", "We should ship it tomorrow."))),
    ("claude", chr(10).join(("整理后的文字如下：", "We should ship it tomorrow."))),
    ("local", chr(10).join(("Sure, here is the cleaned transcript:", "We should ship it tomorrow."))),
    ("local", chr(10).join(("Sure, here's the cleaned transcript:", "We should ship it tomorrow."))),
    ("local", chr(10).join(("Here is the cleaned-up transcript:", "We should ship it tomorrow."))),
    ("claude", chr(10).join(("I'll clean up this transcript:", "We should ship it tomorrow."))),
    ("local", chr(10).join(("I'll clean up this transcript:", "We should ship it tomorrow."))),
])
def test_common_chinese_and_english_intro_lines_are_rejected(mode, output, monkeypatch):
    monkeypatch.setenv("ANTHROPIC_API_KEY", "test-placeholder-not-a-real-key")
    raw = "We should ship it tomorrow."
    result = clean_text(raw, mode, transport=lambda *args: _response(mode, output))

    assert result["status"] == "fallback"
    assert result["text"] == raw
    assert result["error"] == "unexpected_preface"


@pytest.mark.parametrize("mode", ["claude", "local"])
@pytest.mark.parametrize("raw,output", [
    ("我会整理这段转录内容", "我会整理这段转录内容。"),
    ("整理后的文字如下", "整理后的文字如下："),
    ("下面这段录音内容请先保存", "下面这段录音内容，请先保存。"),
])
def test_user_original_intro_like_words_can_receive_punctuation_cleanup(mode, raw, output):
    if mode == "claude":
        response = {"stop_reason": "end_turn", "content": [{"type": "text", "text": output}]}
    else:
        response = {"choices": [{"finish_reason": "stop", "message": {"content": output}}]}

    assert validate_response(response, mode, raw) == output
