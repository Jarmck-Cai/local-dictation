import { strict as assert } from "node:assert";
import type { MeetingSummary } from "../../lib/meetings";
import { meetingCaptureReason } from "./meetingState";
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
  ["meeting_capture_device_removed", "device"],
  ["meeting_capture_queue_full", "queue"],
  ["meeting_io", "storage"],
  ["meeting_capture_recovered", "unknown"],
  ["private/path and transcript", "unknown"],
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
}
console.log("Actual capture notice component, zh/en static rendering: PASS");
