import { strict as assert } from "node:assert";
import type { MeetingSummary } from "../../lib/meetings";
import { meetingCaptureReason, meetingErrorKey } from "./meetingState";
import { readFileSync } from "node:fs";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createInstance } from "i18next";
import { I18nextProvider } from "react-i18next";
import { MeetingCaptureNotice } from "./MeetingCaptureNotice";
import { meetingMessages } from "../../i18n/meeting-translations";

const row = (
  cause?: string,
  status: MeetingSummary["status"] = "interrupted",
): MeetingSummary => ({
  id: "synthetic",
  title: "Synthetic",
  status,
  created_at_ms: 0,
  duration_ms: 0,
  error: null,
  capture_error: cause,
  progress_percent: null,
  microphone_name: null,
  playback_name: null,
});
for (const [code, reason] of [
  ["meeting_atomic_replace_win32_5", "fileAccess"],
  ["meeting_atomic_replace", "stateSave"],
  ["meeting_capture_get_buffer_88890004", "device"],
  ["meeting_capture_next_packet_88890026", "device"],
  ["meeting_capture_stream_health_80004005", "capture"],
  ["meeting_capture_packet_too_large", "capture"],
  ["meeting_capture_nonfinite_sample", "capture"],
  ["meeting_capture_frame_count_invalid", "capture"],
  ["meeting_capture_queue_full", "queue"],
  ["meeting_io", "storage"],
  ["meeting_capture_recovered", "unknown"],
  ["private/path and transcript", "unknownSaved"],
] as const)
  assert.equal(meetingCaptureReason(row(code))?.reason, reason);
assert.equal(
  meetingCaptureReason(row("meeting_io", "complete"))?.reason,
  "storage",
);
assert.equal(meetingCaptureReason(row())?.reason, "unknown");
assert.equal(
  meetingCaptureReason({ ...row(), error: "meeting_io" })?.reason,
  "storage",
);
assert.equal(meetingCaptureReason(row(undefined, "complete")), null);
assert.equal(
  JSON.stringify(
    meetingCaptureReason(row("private/path and transcript")),
  ).includes("private"),
  false,
);
console.log("Meeting capture reason whitelist: PASS");

const productionCapture = readFileSync(
  new URL("../../../src-tauri/src/meetings/capture.rs", import.meta.url),
  "utf8",
);
const operations = new Set(
  [...productionCapture.matchAll(/os_error\("([a-z_]+)"/g)].map((m) => m[1]),
);
assert.equal(operations.size, 28);
for (const operation of operations) {
  const capture = `meeting_capture_${operation}_80004005`;
  const device = `meeting_capture_${operation}_88890004`;
  assert.deepEqual(meetingCaptureReason(row(capture)), {
    reason: "capture",
    code: capture,
  });
  assert.deepEqual(meetingCaptureReason(row(device)), {
    reason: "device",
    code: device,
  });
  assert.equal(meetingErrorKey(capture), "capture");
  assert.equal(meetingErrorKey(device), "devices");
}
assert.deepEqual(meetingCaptureReason(row("meeting_capture_start_ABCDEF01")), {
  reason: "capture",
  code: "meeting_capture_start_abcdef01",
});

for (const code of [
  "meeting_capture_get_buffer_88890004",
  "meeting_capture_stream_health_80004005",
  "meeting_capture_nonfinite_sample",
]) {
  assert.equal(meetingCaptureReason(row(code))?.code, code);
}
for (const code of [
  "meeting_capture_private_path_88890004",
  "meeting_capture_get_buffer_88890004/body",
  "meeting_capture_get_buffer_8889000",
  "meeting_capture_get_buffer_8889000g",
  "meeting_capture_get_buffer_888900040",
  "meeting_capture_get_buffer_88890004\n",
]) {
  const reason = meetingCaptureReason(row(code));
  assert.equal(reason?.reason, "unknownSaved");
  assert.equal(reason?.code, undefined);
}

for (const language of ["zh", "en"] as const) {
  const i18n = createInstance();
  await i18n.init({
    lng: language,
    resources: {
      [language]: { translation: { meetings: meetingMessages[language] } },
    },
  });
  const render = (meeting: MeetingSummary) =>
    renderToStaticMarkup(
      createElement(
        I18nextProvider,
        { i18n },
        createElement(MeetingCaptureNotice, { meeting }),
      ),
    );
  assert.match(
    render(row("meeting_atomic_replace_win32_5")),
    /meeting_atomic_replace_win32_5/,
  );
  assert.match(render(row("meeting_io", "complete")), /role="status"/);
  assert.ok(
    render(row()).includes(meetingMessages[language].captureReasons.unknown),
  );
  assert.equal(
    render(row("private/path and transcript")).includes("private/path"),
    false,
  );
  assert.equal(render(row(undefined, "complete")), "");
  for (const code of [
    "meeting_capture_get_buffer_88890004",
    "meeting_capture_stream_health_80004005",
    "meeting_capture_packet_too_large",
    "meeting_capture_nonfinite_sample",
    "meeting_capture_frame_count_invalid",
  ]) {
    const html = render(row(code));
    assert.ok(html.includes(code));
    assert.equal(
      html.includes(meetingMessages[language].captureReasons.unknown),
      false,
    );
  }
}
console.log("Actual capture notice component, zh/en static rendering: PASS");
