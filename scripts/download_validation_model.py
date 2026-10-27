"""Download the documented sherpa-onnx SenseVoice INT8 validation artifact."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path
import shutil
import tarfile
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
NAME = "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17"
URL = f"https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/{NAME}.tar.bz2"
ALLOWED = {"model.int8.onnx", "tokens.txt", "LICENSE", "README.md"}


def main() -> None:
    target = ROOT / "models" / NAME
    target.mkdir(parents=True, exist_ok=True)
    vad = target / "silero_vad.onnx"
    if not vad.exists():
        print("Downloading official Silero VAD artifact...", flush=True)
        urllib.request.urlretrieve(
            "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx", vad.with_suffix(".partial"))
        vad.with_suffix(".partial").replace(vad)
    if (target / "model.int8.onnx").is_file() and (target / "download.json").is_file():
        print(f"Model already downloaded: {target}", flush=True)
        return
    cache = ROOT / ".cache" / "downloads"
    cache.mkdir(parents=True, exist_ok=True)
    archive = cache / f"{NAME}.tar.bz2"
    if not archive.exists():
        partial = archive.with_suffix(".partial")
        print("Downloading official SenseVoice INT8 artifact...", flush=True)
        request = urllib.request.Request(URL, headers={"User-Agent": "local-dictation-validation"})
        with urllib.request.urlopen(request, timeout=90) as response, partial.open("wb") as output:
            shutil.copyfileobj(response, output, length=1024 * 1024)
        partial.replace(archive)
    target.mkdir(parents=True, exist_ok=True)
    extracted = []
    with tarfile.open(archive, "r:bz2") as bundle:
        for member in bundle:
            if not member.isfile():
                continue
            parts = Path(member.name).parts
            if not parts or parts[0] != NAME:
                continue
            relative = Path(*parts[1:])
            if relative.as_posix() not in ALLOWED and not (
                len(relative.parts) == 2 and relative.parts[0] == "test_wavs" and relative.suffix == ".wav"
            ):
                continue
            destination = target / relative
            if not destination.resolve().is_relative_to(target.resolve()):
                raise ValueError("Archive member escapes the model directory")
            destination.parent.mkdir(parents=True, exist_ok=True)
            with bundle.extractfile(member) as source, destination.open("wb") as output:
                shutil.copyfileobj(source, output)
            extracted.append(relative.as_posix())
    with archive.open("rb") as source:
        digest = hashlib.file_digest(source, "sha256").hexdigest()
    manifest = {"source_url": URL, "archive_sha256": digest, "files": extracted,
                "note": "SHA-256 records the downloaded bytes; it is not an independently verified upstream checksum."}
    (target / "download.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    print(json.dumps({"directory": str(target), "archive_sha256": digest, "files": extracted}, indent=2), flush=True)


if __name__ == "__main__":
    main()
