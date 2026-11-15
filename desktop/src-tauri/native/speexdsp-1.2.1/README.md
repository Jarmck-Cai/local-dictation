# SpeexDSP 1.2.1 AEC subset

Unmodified files copied from the official Xiph SpeexDSP 1.2.1 source release:

- Source: https://downloads.xiph.org/releases/speex/speexdsp-1.2.1.tar.gz
- Archive SHA-256: `8c777343e4a6399569c72abc38a95b24db56882c83dbdb6c6424a5f4aeb54d3d`
- Official fixed tag: https://github.com/xiph/speexdsp/tree/SpeexDSP-1.2.1
- License: BSD terms in `COPYING` and the original per-file headers. `smallft.c`
  includes its original permissive Ogg/Vorbis FFT notice. Preserve all of these
  and reproduce the notices in binary distributions.

Only the MDF echo canceller, SmallFT and their required headers are included.
There is no Speex codec, jitter buffer, preprocessor, AGC, resampler, SIMD patch,
model, eSpeak executable or speech fixture in this product source subset.
`build_meeting_aec.rs` uses floating point, SmallFT, static C linkage and the
target's default CRT. The normalized Rust API quantizes to the upstream i16
interface and validates all frame lengths and values before calling C.

SpeexDSP requires synchronized near-end and reference samples. Timestamp
alignment and drift correction belong to the offline reader, before the AEC.
The standalone synthetic gates in `scripts/check-meeting-aec.py` are engineering
checks and are not evidence of a real room, microphone or meeting service test.
