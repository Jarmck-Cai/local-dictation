from __future__ import annotations

import math
import os
from pathlib import Path
import re
import time
import unicodedata

ROOT = Path(__file__).resolve().parents[1]
MODEL_DIR = ROOT / "models" / "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17"
RECORDINGS = ROOT / "recordings" / "validation"
ENV_NAMES = {"ANTHROPIC_API_KEY", "CLAUDE_MODEL", "CLAUDE_API_BASE_URL", "LOCAL_LLM_BASE_URL",
             "LOCAL_LLM_MODEL", "LOCAL_LLM_API_KEY"}


def load_local_env(path: Path = ROOT / ".env.local") -> None:
    if not path.is_file():
        return
    for line in path.read_text(encoding="utf-8-sig").splitlines():
        key, separator, value = line.partition("=")
        if separator and key.strip() in ENV_NAMES:
            if not os.environ.get(key.strip()):
                os.environ[key.strip()] = value.strip().strip("\"'")


def mixed_error_rate(reference: str, hypothesis: str) -> float | None:
    def tokenize(value):
        return re.findall(r"[\u3400-\u9fff]|[a-z]+(?:'[a-z]+)?|\d+(?:[.,]\d+)*",
                          unicodedata.normalize("NFKC", value).lower())
    expected, actual = tokenize(reference), tokenize(hypothesis)
    if not expected:
        return None
    row = list(range(len(actual) + 1))
    for i, wanted in enumerate(expected, 1):
        next_row = [i]
        for j, got in enumerate(actual, 1):
            next_row.append(min(next_row[-1] + 1, row[j] + 1, row[j - 1] + (wanted != got)))
        row = next_row
    return row[-1] / len(expected)


def percentile(values: list[float], fraction: float) -> float | None:
    if not values:
        return None
    ordered = sorted(values)
    position = (len(ordered) - 1) * fraction
    lower, upper = math.floor(position), math.ceil(position)
    return ordered[lower] + (ordered[upper] - ordered[lower]) * (position - lower)


def prune_recordings(root: Path = RECORDINGS, now: float | None = None) -> list[str]:
    if not root.exists():
        return []
    if root.is_symlink():
        raise ValueError("Recording root must not be a symbolic link")
    resolved = root.resolve()
    cutoff = (time.time() if now is None else now) - 7 * 24 * 60 * 60
    removed = []
    for file in root.glob("*.wav"):
        if file.is_symlink() or not file.resolve().is_relative_to(resolved):
            continue
        if file.stat().st_mtime <= cutoff:
            try:
                file.unlink()
                removed.append(file.name)
            except OSError:
                # Keep failed files discoverable for the next startup/periodic sweep.
                continue
    return removed
