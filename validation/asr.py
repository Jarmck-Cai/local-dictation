from __future__ import annotations

from pathlib import Path
import wave

import numpy as np
import sherpa_onnx

from .core import MODEL_DIR


class SenseVoice:
    def __init__(self, threads: int = 4, model_dir: Path = MODEL_DIR, segmentation: str = "vad"):
        self.segmentation = segmentation
        self.last_segments = []
        self.recognizer = sherpa_onnx.OfflineRecognizer.from_sense_voice(
            model=str(model_dir / "model.int8.onnx"), tokens=str(model_dir / "tokens.txt"),
            num_threads=threads, provider="cpu", language="auto", use_itn=True, debug=False)
        self.vad_config = sherpa_onnx.VadModelConfig()
        self.vad_config.silero_vad.model = str(model_dir / "silero_vad.onnx")
        self.vad_config.silero_vad.min_silence_duration = 0.35
        self.vad_config.silero_vad.max_speech_duration = 15
        self.vad_config.sample_rate = 16000

    def decode(self, samples: np.ndarray, sample_rate: int) -> str:
        self.last_segments = []
        if self.segmentation == "whole" or len(samples) <= sample_rate * 20:
            self.last_segments.append({"start_seconds": 0, "duration_seconds": len(samples) / sample_rate})
            return self._decode(samples, sample_rate)
        if sample_rate != 16000:
            raise ValueError("长录音 VAD 验证需要 16000 Hz WAV")
        vad = sherpa_onnx.VoiceActivityDetector(self.vad_config, buffer_size_in_seconds=30)
        window = self.vad_config.silero_vad.window_size
        texts = []

        def drain():
            while not vad.empty():
                # front is a borrowed buffer; copy before mutating the VAD queue.
                segment = np.array(vad.front.samples, dtype=np.float32, copy=True)
                self.last_segments.append({"start_seconds": vad.front.start / sample_rate,
                                           "duration_seconds": len(segment) / sample_rate})
                vad.pop()
                text = self._decode(segment, sample_rate)
                if text:
                    texts.append(text)

        for offset in range(0, len(samples), window):
            frame = samples[offset:offset + window]
            if len(frame) < window:
                frame = np.pad(frame, (0, window - len(frame)))
            vad.accept_waveform(frame)
            drain()
        vad.flush()
        drain()
        return " ".join(texts)

    def _decode(self, samples: np.ndarray, sample_rate: int) -> str:
        stream = self.recognizer.create_stream()
        stream.accept_waveform(sample_rate, np.asarray(samples, dtype=np.float32))
        self.recognizer.decode_stream(stream)
        return stream.result.text.strip()


def read_wav(path: Path) -> tuple[np.ndarray, int]:
    with wave.open(str(path), "rb") as audio:
        if audio.getsampwidth() != 2 or audio.getcomptype() != "NONE":
            raise ValueError("验证工具需要未压缩的 16-bit PCM WAV")
        rate, channels = audio.getframerate(), audio.getnchannels()
        samples = np.frombuffer(audio.readframes(audio.getnframes()), dtype="<i2").astype(np.float32)
        return samples.reshape(-1, channels).mean(axis=1) / 32768, rate
