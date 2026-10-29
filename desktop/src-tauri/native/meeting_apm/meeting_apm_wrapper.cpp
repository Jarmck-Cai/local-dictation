#include <algorithm>
#include <array>
#include <cmath>
#include <cstdio>
#include <memory>
#include <new>
#include "api/audio/audio_processing.h"

// Prototype only. No changes to upstream APM algorithms. Single-threaded,
// sequential 160-sample mono capture/render pairs at 16 kHz.
namespace {
constexpr size_t kFrame = 160;
struct State {
  rtc::scoped_refptr<webrtc::AudioProcessing> apm;
  webrtc::StreamConfig stream{16000, 1};
  State() {
    webrtc::AudioProcessing::Config cfg;
    cfg.echo_canceller.enabled = true;
    cfg.echo_canceller.mobile_mode = false;
    cfg.echo_canceller.enforce_high_pass_filtering = false;
    cfg.echo_canceller.export_linear_aec_output = false;
    cfg.high_pass_filter.enabled = false;
    cfg.noise_suppression.enabled = false;
    cfg.gain_controller1.enabled = false;
    cfg.gain_controller2.enabled = false;
    apm = webrtc::AudioProcessingBuilder().SetConfig(cfg).Create();
    if (!apm || apm->Initialize() != 0) throw std::bad_alloc();
  }
};
bool valid(const float* x, size_t n) {
  if (!x || n != kFrame) return false;
  for (size_t i = 0; i < n; ++i)
    if (!std::isfinite(x[i]) || std::abs(x[i]) > 1.0f) return false;
  return true;
}
}
#define ABI extern "C" __declspec(dllexport)
ABI void* meeting_apm_create(int linear_diagnostic) noexcept {
  // This candidate exposes only complete upstream AEC3 output. APM 2.1's
  // public export_linear_aec_output flag alone does not initialize the AEC3
  // linear framer; unsupported diagnostic requests are rejected safely.
  if (linear_diagnostic != 0) return nullptr;
  try { return new State(); } catch (...) { return nullptr; }
}
ABI void meeting_apm_destroy(void* handle) noexcept {
  try { delete static_cast<State*>(handle); } catch (...) {}
}
ABI int meeting_apm_reset(void* handle) noexcept {
  if (!handle) return -100;
  try {
    auto* p = static_cast<State*>(handle);
    State replacement;
    p->apm = replacement.apm;
    return 0;
  } catch (...) { return -103; }
}
ABI int meeting_apm_process(void* handle, const float* near, size_t near_n,
                            const float* render, size_t render_n,
                            float* out, size_t out_n) noexcept {
  // Validate the entire call before the upstream state advances or out changes.
  if (!handle || !out) return -100;
  if (!valid(near, near_n) || !valid(render, render_n) || out_n != kFrame)
    return -101;
  try {
    auto& s = *static_cast<State*>(handle);
    const float* near_channels[] = {near};
    const float* render_channels[] = {render};
    std::array<float, kFrame> temp{};
    float* output_channels[] = {temp.data()};
    int status = s.apm->AnalyzeReverseStream(render_channels, s.stream);
    if (status != 0) return status;
    status = s.apm->set_stream_delay_ms(0);
    if (status != 0) return status;
    status = s.apm->ProcessStream(near_channels, s.stream, s.stream, output_channels);
    if (status != 0) return status;
    for (float x : temp) if (!std::isfinite(x)) return -102;
    std::copy(temp.begin(), temp.end(), out);
    return 0;
  } catch (...) { return -103; }
}


