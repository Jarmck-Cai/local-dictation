import numpy as np
import pytest

from validation.asr import SenseVoice, read_wav
from validation.core import MODEL_DIR


@pytest.mark.skipif(not (MODEL_DIR / "silero_vad.onnx").is_file(), reason="Local models not installed")
def test_minute_audio_does_not_collapse_repeated_english_phrases():
    zh, rate = read_wav(MODEL_DIR / "test_wavs" / "zh.wav")
    en, _ = read_wav(MODEL_DIR / "test_wavs" / "en.wav")
    unit = np.concatenate([zh, np.zeros(int(rate * 0.35), dtype=np.float32), en])
    audio = np.tile(unit, 5)[:rate * 60]
    engine = SenseVoice(threads=4)
    text = engine.decode(audio, rate)
    assert text.lower().count("tribal") >= 3, text
