# Meeting echo processor

`meeting_apm_wrapper.cpp` is a narrow C interface to the complete AEC3 output
from the unmodified WebRTC Audio Processing 2.1 source distribution (WebRTC M131).
It accepts sequential 160-sample, 16 kHz mono float frames and owns one processor
per recording epoch. AGC, noise suppression and high-pass filtering are disabled;
AEC3 uses its upstream suppression settings. No linear-output mode is exposed.

The fixed processing delay is 128 samples. Callers compensate that delay, flush
the tail, keep the original tracks, and stop processing after native errors.
State and C++ types never cross the C interface. Each handle has one owner and
is used on one thread.

Run `scripts/prepare-meeting-apm.ps1` before Cargo. The script requires Python
3.12+ with pip and Visual Studio 2022 x64 C++ tools. Meson and Ninja are installed
only into the project cache. Five downloads have fixed SHA-256 digests; upstream
source members, actual target/CRT and the resulting COFF archive are checked.
`-Offline` reuses verified downloads. No download runs inside Cargo.

The prepared cache defaults to `.cache/native/webrtc-apm-2.1-msvc-x64-md`.
`MEETING_APM_DIR` can select another prepared cache. Cargo verifies its manifest
and library digest, builds this wrapper and links the static APM archive plus
`winmm`. Build libraries, tools and source caches are excluded from source Git
and the installer. The complete upstream notices are distributed in
`resources/webrtc-apm-licenses/`.

Official source archives:

- [WebRTC Audio Processing 2.1](https://freedesktop.org/software/pulseaudio/webrtc-audio-processing/webrtc-audio-processing-2.1.tar.gz): `35e86b986d02ea15f3d04741a1a5a735ba399bc0fac0ee089c39480e35fc3253`
- [Abseil 20240722.0](https://github.com/abseil/abseil-cpp/releases/download/20240722.0/abseil-cpp-20240722.0.tar.gz): `f50e5ac311a81382da7fa75b97310e4b9006474f9560ac46f54a9967f07d4ae3`
- [Meson WrapDB Abseil build definitions](https://wrapdb.mesonbuild.com/v2/abseil-cpp_20240722.0-3/get_patch): `12dd8df1488a314c53e3751abd2750cf233b830651d168b6a9f15e7d0cf71f7b`
