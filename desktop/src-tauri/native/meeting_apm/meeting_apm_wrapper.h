#ifndef QINGSHU_MEETING_APM_WRAPPER_H
#define QINGSHU_MEETING_APM_WRAPPER_H
#include <stddef.h>
#ifdef __cplusplus
extern "C" {
#define MEETING_APM_NOEXCEPT noexcept
#else
#define MEETING_APM_NOEXCEPT
#endif
// One owner/thread per handle. 16 kHz mono, 160 normalized float samples.
// Complete AEC3 output only. reserved must be 0; otherwise returns NULL.
void* meeting_apm_create(int reserved) MEETING_APM_NOEXCEPT;
void meeting_apm_destroy(void* handle) MEETING_APM_NOEXCEPT;
int meeting_apm_reset(void* handle) MEETING_APM_NOEXCEPT;
// Input may alias output; capture/render input is not modified. Entire inputs
// and lengths are checked before advancing state. Output is written on success
// only. A native processing failure may advance state: reset before retrying.
// 0=success; -100=null handle/output; -101=null capture/render input,
// invalid length/nonfinite/out-of-range input;
// -102=nonfinite native output; -103=allocation/C++ exception; other=APM error.
int meeting_apm_process(void* handle, const float* capture, size_t capture_n,
                        const float* render, size_t render_n,
                        float* output, size_t output_n) MEETING_APM_NOEXCEPT;
#ifdef __cplusplus
}
#endif
#undef MEETING_APM_NOEXCEPT
#endif
