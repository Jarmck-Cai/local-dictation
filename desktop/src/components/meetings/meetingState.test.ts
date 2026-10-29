import assert from "node:assert/strict";
import type {
  MeetingsTransport,
  MeetingSummary,
  MeetingTranscript,
} from "../../lib/meetings";
import {
  MeetingsState,
  meetingErrorKey,
  meetingWarningKeys,
} from "./meetingState";

const summary = (
  id: string,
  status: MeetingSummary["status"],
): MeetingSummary => ({
  id,
  title: `Meeting ${id}`,
  status,
  created_at_ms: 1000,
  duration_ms: 120000,
  error: null,
  progress_percent: status === "processing" ? 20 : null,
  microphone_name: "Test microphone",
  playback_name: "Test speakers",
});
const transcript = (id: string): MeetingTranscript => ({
  id,
  complete: true,
  segments: [
    { id: 0, source: "microphone", start_ms: 0, end_ms: 500, text: id },
  ],
  warnings: [],
});
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
function transport(
  overrides: Partial<MeetingsTransport> = {},
): MeetingsTransport {
  return {
    devices: async () => [
      { id: "mic", name: "Mic", source: "microphone", is_default: true },
      { id: "speaker", name: "Speaker", source: "remote", is_default: true },
    ],
    list: async () => [],
    current: async () => null,
    start: async () => summary("new", "recording"),
    stop: async (id) => summary(id, "recorded"),
    transcribe: async (id) => summary(id, "processing"),
    cancelTranscription: async (id) => summary(id, "cancelled"),
    transcript: async (id) => transcript(id),
    delete: async () => {},
    export: async () => {},
    ...overrides,
  };
}

// Successful retry transcribes retained audio without erasing the recording interruption.
{
  const id = "interrupted-warning";
  let saved: MeetingTranscript | null = null;
  const state = new MeetingsState(
    transport({ transcript: async () => saved }),
    () => {},
  );
  state.receive({
    ...summary(id, "interrupted"),
    error: "meeting_capture_microphone_stalled",
  });
  await state.select(id);
  await state.retry(id);
  saved = {
    ...transcript(id),
    warnings: [
      "meeting_alignment_incomplete_recording",
      "meeting_alignment_initial_discontinuity",
      "meeting_alignment_incomplete_recording",
      "meeting_apm_input_clamped",
      "meeting_apm_input_clamped",
    ],
  };
  state.receive(summary(id, "complete"));
  await state.select(id);
  assert.equal(state.snapshot.meetings[0]?.status, "complete");
  assert.equal(state.snapshot.meetings[0]?.error, null);
  assert.equal(state.snapshot.transcript?.complete, true);
  assert.ok(state.snapshot.transcript?.segments.length);
  assert.deepEqual(
    meetingWarningKeys(state.snapshot.transcript?.warnings ?? []),
    ["meetings.warningIncompleteRecording", "meetings.warningInputClamped"],
    "completion must retain distinct, deduplicated recording and volume warnings",
  );
}

// Validated clock observations alone must not suggest missing meeting audio.
{
  const state = new MeetingsState(
    transport({
      transcript: async (id) => ({
        ...transcript(id),
        warnings: [
          "meeting_alignment_clock_timestamp_jitter",
          "meeting_alignment_initial_discontinuity",
          "meeting_alignment_remote_resume_discontinuity",
          "meeting_alignment_remote_no_playback",
          "meeting_alignment_single_anchor_nominal_rate",
          "meeting_alignment_short_span_nominal_rate",
          "meeting_alignment_boundary_padding",
        ],
      }),
    }),
    () => {},
  );
  state.receive(summary("validated-clock-warning", "complete"));
  await state.select("validated-clock-warning");
  assert.deepEqual(
    meetingWarningKeys(state.snapshot.transcript?.warnings ?? []),
    [],
  );
}

// Unknown details stay neutral and cannot hide a known recording interruption.
{
  const state = new MeetingsState(
    transport({
      transcript: async (id) => ({
        ...transcript(id),
        warnings: [
          "private detail C:/Users/Example/meeting.wav",
          "meeting_alignment_incomplete_recording",
          "unknown_warning",
          "toString",
        ],
      }),
    }),
    () => {},
  );
  state.receive(summary("unknown-warning", "complete"));
  await state.select("unknown-warning");
  assert.deepEqual(
    meetingWarningKeys(state.snapshot.transcript?.warnings ?? []),
    ["meetings.warnings", "meetings.warningIncompleteRecording"],
  );
}

// A second click before stop resolves must neither stop twice nor start ASR early.
{
  const stopped = deferred<MeetingSummary>();
  const calls: string[] = [];
  const state = new MeetingsState(
    transport({
      stop: (id) => {
        calls.push(`stop:${id}`);
        return stopped.promise;
      },
      transcribe: async (id) => {
        calls.push(`transcribe:${id}`);
        return summary(id, "processing");
      },
    }),
    () => {},
  );
  state.receive(summary("a", "recording"));
  const finishing = state.finish("a");
  await state.finish("a");
  assert.deepEqual(calls, ["stop:a"]);
  assert.equal(state.snapshot.busy, "finish");
  stopped.resolve(summary("a", "recorded"));
  await finishing;
  assert.deepEqual(calls, ["stop:a", "transcribe:a"]);
  assert.equal(state.snapshot.active?.status, "processing");
}

// Failed transcription starts leave the saved recording available for a later retry.
{
  let attempts = 0;
  const state = new MeetingsState(
    transport({
      transcribe: async (id) => {
        if (++attempts === 1) throw "meeting_model_unavailable";
        return summary(id, "processing");
      },
    }),
    () => {},
  );
  state.receive(summary("a", "recording"));
  await state.finish("a");
  assert.equal(state.snapshot.active, null);
  assert.equal(state.snapshot.meetings[0]?.status, "recorded");
  assert.equal(state.snapshot.error, "model");
  await state.retry("a");
  assert.equal(state.snapshot.meetings[0]?.status, "processing");
}

// A failed stop cannot transcribe audio while the native recorder may still run.
{
  let transcribed = false;
  const state = new MeetingsState(
    transport({
      stop: async () => {
        throw "meeting_capture_stop_failed";
      },
      transcribe: async (id) => {
        transcribed = true;
        return summary(id, "processing");
      },
    }),
    () => {},
  );
  state.receive(summary("a", "recording"));
  await state.finish("a");
  assert.equal(transcribed, false);
  assert.equal(state.snapshot.active?.id, "a");
  assert.equal(state.snapshot.busy, null);
}

// A late transcript response for A must not appear underneath newly selected B.
{
  const old = deferred<MeetingTranscript | null>();
  const state = new MeetingsState(
    transport({
      transcript: (id) =>
        id === "a" ? old.promise : Promise.resolve(transcript(id)),
    }),
    () => {},
  );
  const first = state.select("a");
  await state.select("b");
  old.resolve(transcript("a"));
  await first;
  assert.equal(state.snapshot.selectedId, "b");
  assert.equal(state.snapshot.transcript?.id, "b");
}

// A list refresh begun before completion cannot turn a finished row back to processing.
{
  const old = deferred<MeetingSummary[]>();
  const state = new MeetingsState(
    transport({ list: () => old.promise }),
    () => {},
  );
  state.receive(summary("a", "processing"));
  const loading = state.refreshList();
  state.receive(summary("a", "complete"));
  old.resolve([summary("a", "processing")]);
  await loading;
  assert.equal(state.snapshot.meetings[0]?.status, "complete");
  assert.equal(state.snapshot.active, null);
}

// Background completion may arrive before the initial transcribe command returns.
{
  const started = deferred<MeetingSummary>();
  const state = new MeetingsState(
    transport({ transcribe: () => started.promise }),
    () => {},
  );
  state.receive(summary("a", "recorded"));
  const retry = state.retry("a");
  state.receive(summary("a", "complete"));
  started.resolve(summary("a", "processing"));
  await retry;
  assert.equal(state.snapshot.meetings[0]?.status, "complete");
  assert.equal(state.snapshot.active, null);
}

// A removed device and an unset device both require an explicit new selection.
{
  let starts = 0;
  const state = new MeetingsState(
    transport({
      start: async () => {
        starts++;
        return summary("a", "recording");
      },
    }),
    () => {},
  );
  await state.loadDevices();
  await state.start({ title: "A", microphoneId: "", renderId: "speaker" });
  await state.start({
    title: "A",
    microphoneId: "removed",
    renderId: "speaker",
  });
  assert.equal(starts, 0);
  assert.equal(state.snapshot.error, "devices");
  await state.start({ title: "A", microphoneId: "mic", renderId: "speaker" });
  assert.equal(starts, 1);
}

// Export cancellation never writes; active processing cannot be deleted.
{
  let exports = 0;
  let deletes = 0;
  const state = new MeetingsState(
    transport({
      export: async () => {
        exports++;
      },
      delete: async () => {
        deletes++;
      },
    }),
    () => {},
  );
  state.receive(summary("a", "recorded"));
  await state.export("a", "audio", "en", async () => null);
  assert.equal(exports, 0);
  state.receive(summary("a", "processing"));
  await state.remove("a");
  assert.equal(deletes, 0);
}

assert.equal(meetingErrorKey("meeting_io"), "storage");
assert.equal(
  meetingErrorKey("C:\\private\\recording.wav: rejected"),
  "generic",
);
assert.equal(
  meetingErrorKey({ message: "meeting_model_unavailable" }),
  "generic",
);

// A model compatibility failure keeps the recording ready for an explicit retry.
{
  const state = new MeetingsState(
    transport({
      transcribe: async () => {
        throw "meeting_model_unsupported";
      },
    }),
    () => {},
  );
  state.receive(summary("model-contract", "recorded"));
  await state.retry("model-contract");
  assert.equal(state.snapshot.error, "modelUnsupported");
  assert.equal(state.snapshot.meetings[0]?.status, "recorded");
  assert.equal(state.snapshot.active, null);
}

// A stopped transcription job does not imply a failed recording or missing model.
{
  const state = new MeetingsState(transport(), () => {});
  state.receive({
    ...summary("processing-contract", "failed"),
    error: "meeting_processing_interrupted",
  });
  assert.equal(
    meetingErrorKey(state.snapshot.meetings[0]?.error),
    "processing",
  );
  await state.retry("processing-contract");
  assert.equal(state.snapshot.meetings[0]?.status, "processing");
}

// A current-status poll waiting across a completion event cannot restore recording.
{
  const old = deferred<MeetingSummary | null>();
  const state = new MeetingsState(
    transport({ current: () => old.promise }),
    () => {},
  );
  state.receive(summary("a", "recording"));
  const loading = state.loadCurrent();
  state.receive(summary("a", "complete"));
  old.resolve(summary("a", "recording"));
  await loading;
  assert.equal(state.snapshot.active, null);
  assert.equal(state.snapshot.meetings[0]?.status, "complete");
}

// Idle screens and overlapping timer ticks must never create extra status reads.
{
  let reads = 0;
  let transcriptReads = 0;
  const pending = deferred<MeetingSummary | null>();
  const state = new MeetingsState(
    transport({
      current: () => {
        reads++;
        return pending.promise;
      },
      transcript: async () => {
        transcriptReads++;
        return null;
      },
    }),
    () => {},
  );
  await state.pollCurrent();
  assert.equal(reads, 0);
  state.receive(summary("a", "recording"));
  const tick = state.pollCurrent();
  await state.pollCurrent();
  assert.equal(reads, 1);
  pending.resolve(summary("a", "recording"));
  await tick;
  assert.equal(transcriptReads, 0);
}

// Navigating away while stop is saving must still honor "finish and transcribe".
{
  const stopped = deferred<MeetingSummary>();
  let transcriptions = 0;
  const state = new MeetingsState(
    transport({
      stop: () => stopped.promise,
      transcribe: async (id) => {
        transcriptions++;
        return summary(id, "processing");
      },
    }),
    () => {},
  );
  state.receive(summary("a", "recording"));
  const finishing = state.finish("a");
  state.dispose();
  stopped.resolve(summary("a", "recorded"));
  await finishing;
  assert.equal(transcriptions, 1);
}

// A confirmed save dialog remains an authorized export after navigating away.
{
  const destination = deferred<string | null>();
  const exports: unknown[][] = [];
  const state = new MeetingsState(
    transport({
      export: async (...args) => {
        exports.push(args);
      },
    }),
    () => {},
  );
  state.receive(summary("export-after-navigation", "complete"));
  const exporting = state.export(
    "export-after-navigation",
    "text",
    "zh",
    () => destination.promise,
  );
  state.dispose();
  destination.resolve("C:/MockExport/confirmed.txt");
  await exporting;
  assert.deepEqual(exports, [
    ["export-after-navigation", "C:/MockExport/confirmed.txt", "text", "zh"],
  ]);
}

// A late start response must keep the user's newer history selection.
{
  const started = deferred<MeetingSummary>();
  const state = new MeetingsState(
    transport({ start: () => started.promise }),
    () => {},
  );
  state.receive(summary("history", "complete"));
  await state.loadDevices();
  const starting = state.start({
    title: "New",
    microphoneId: "mic",
    renderId: "speaker",
  });
  await state.select("history");
  started.resolve(summary("new-recording", "recording"));
  await starting;
  assert.equal(state.snapshot.selectedId, "history");
  assert.equal(state.snapshot.transcript?.id, "history");
  assert.equal(state.snapshot.active?.id, "new-recording");
}

// A's export finishing late must not announce export success in B's details.
{
  const saved = deferred<void>();
  const state = new MeetingsState(
    transport({ export: () => saved.promise }),
    () => {},
  );
  state.receive(summary("export-a", "complete"));
  state.receive(summary("view-b", "complete"));
  await state.select("export-a");
  const exporting = state.export(
    "export-a",
    "audio",
    "en",
    async () => "C:/MockExport/a.wav",
  );
  await state.select("view-b");
  saved.resolve();
  await exporting;
  assert.equal(state.snapshot.selectedId, "view-b");
  assert.equal(state.snapshot.exported, false);
}

// Native cancellation acknowledges the request while the current ASR section still runs.
{
  let cancellations = 0;
  const state = new MeetingsState(
    transport({
      cancelTranscription: async (id) => {
        cancellations++;
        return summary(id, "processing");
      },
      current: async () => summary("cancel-pending", "processing"),
    }),
    () => {},
  );
  state.receive(summary("cancel-pending", "processing"));
  await state.cancel("cancel-pending");
  assert.equal(state.snapshot.busy, null);
  assert.equal(state.snapshot.cancellingId, "cancel-pending");
  await state.cancel("cancel-pending");
  assert.equal(cancellations, 1);
  await state.pollCurrent();
  assert.equal(state.snapshot.cancellingId, "cancel-pending");
  state.receive(summary("cancel-pending", "cancelled"));
  assert.equal(state.snapshot.cancellingId, null);
}

// The same in-flight job keeps its cancellation intent when its page remounts.
{
  let cancellations = 0;
  let current: MeetingSummary = summary("cancel-remount", "processing");
  const api = transport({
    cancelTranscription: async () => {
      cancellations++;
      return current;
    },
    current: async () => current,
    list: async () => [current],
  });
  const first = new MeetingsState(api, () => {});
  first.receive(current);
  await first.cancel(current.id);
  first.dispose();
  const returned = new MeetingsState(api, () => {});
  await returned.load();
  assert.equal(returned.snapshot.cancellingId, "cancel-remount");
  await returned.cancel(current.id);
  assert.equal(cancellations, 1);
  current = summary("cancel-remount", "failed");
  await returned.pollCurrent();
  assert.equal(returned.snapshot.cancellingId, null);
  assert.equal(returned.snapshot.active, null);
}

// A rejected cancellation request can be tried again, rather than trapping the UI.
{
  let cancellations = 0;
  const state = new MeetingsState(
    transport({
      cancelTranscription: async (id) => {
        if (++cancellations === 1) throw "meeting_busy";
        return summary(id, "processing");
      },
    }),
    () => {},
  );
  state.receive(summary("cancel-rejected", "processing"));
  await state.cancel("cancel-rejected");
  assert.equal(state.snapshot.cancellingId, null);
  assert.equal(state.snapshot.error, "busy");
  await state.cancel("cancel-rejected");
  assert.equal(cancellations, 2);
  assert.equal(state.snapshot.cancellingId, "cancel-rejected");
  state.receive(summary("cancel-rejected", "complete"));
  assert.equal(state.snapshot.cancellingId, null);
}

// A terminal event racing the cancel acknowledgement cannot restore pending UI.
{
  const acknowledged = deferred<MeetingSummary>();
  const state = new MeetingsState(
    transport({ cancelTranscription: () => acknowledged.promise }),
    () => {},
  );
  state.receive(summary("cancel-race", "processing"));
  const cancelling = state.cancel("cancel-race");
  state.receive(summary("cancel-race", "complete"));
  acknowledged.resolve(summary("cancel-race", "processing"));
  await cancelling;
  assert.equal(state.snapshot.cancellingId, null);
  assert.equal(state.snapshot.active, null);
}

// A previous job's delayed Failed event cannot stop polling its newer retry.
{
  const started = deferred<MeetingSummary>();
  const state = new MeetingsState(
    transport({ transcribe: () => started.promise }),
    () => {},
  );
  state.receive({ ...summary("revision-retry", "failed"), revision: 10 });
  await state.select("revision-retry");
  const retrying = state.retry("revision-retry");
  state.receive({ ...summary("revision-retry", "processing"), revision: 12 });
  state.receive({ ...summary("revision-retry", "failed"), revision: 10 });
  assert.equal(state.snapshot.active?.status, "processing");
  assert.equal(state.snapshot.busy, "transcribe");
  assert.equal(state.snapshot.selectedId, "revision-retry");
  started.resolve({ ...summary("revision-retry", "processing"), revision: 12 });
  await retrying;
}

// Delayed progress cannot restore an active job after its newer terminal event.
{
  const state = new MeetingsState(transport(), () => {});
  state.receive({
    ...summary("revision-complete", "processing"),
    revision: 20,
  });
  state.receive({ ...summary("revision-complete", "complete"), revision: 22 });
  await state.select("revision-complete");
  state.receive({
    ...summary("revision-complete", "processing"),
    revision: 21,
    progress_percent: 80,
  });
  assert.equal(state.snapshot.active, null);
  assert.equal(state.snapshot.meetings[0]?.status, "complete");
  assert.equal(state.snapshot.transcript?.id, "revision-complete");
}

// Old current() results remain stale even when the query began after the newest event.
{
  const state = new MeetingsState(
    transport({
      current: async () => ({
        ...summary("revision-current", "failed"),
        revision: 30,
      }),
    }),
    () => {},
  );
  state.receive({ ...summary("revision-current", "processing"), revision: 32 });
  await state.loadCurrent();
  assert.equal(state.snapshot.active?.revision, 32);
  assert.equal(state.snapshot.meetings[0]?.status, "processing");
}

// A newer native query beats unrelated events that arrived while it was waiting.
{
  const queried = deferred<MeetingSummary | null>();
  const state = new MeetingsState(
    transport({ current: () => queried.promise }),
    () => {},
  );
  state.receive({
    ...summary("revision-new-query", "processing"),
    revision: 40,
  });
  const loading = state.loadCurrent();
  state.receive({ ...summary("revision-unrelated", "complete"), revision: 41 });
  queried.resolve({
    ...summary("revision-new-query", "complete"),
    revision: 42,
  });
  await loading;
  assert.equal(state.snapshot.active, null);
  assert.equal(
    state.snapshot.meetings.find((row) => row.id === "revision-new-query")
      ?.revision,
    42,
  );
}

// Old list rows, including legacy revision zero, cannot overwrite known complete details.
{
  let listed: MeetingSummary[] = [
    { ...summary("revision-list-old", "processing"), revision: 50 },
  ];
  const state = new MeetingsState(
    transport({ list: async () => listed }),
    () => {},
  );
  state.receive({
    ...summary("revision-list-old", "complete"),
    revision: 52,
    title: "Latest title",
  });
  await state.select("revision-list-old");
  await state.refreshList();
  assert.equal(state.snapshot.meetings[0]?.status, "complete");
  assert.equal(state.snapshot.transcript?.id, "revision-list-old");
  listed = [summary("revision-list-old", "failed")];
  await state.refreshList();
  assert.equal(state.snapshot.meetings[0]?.title, "Latest title");
  assert.equal(state.snapshot.meetings[0]?.revision, 52);
}

// A list's newer terminal summary must end active polling and settle cancellation.
{
  const state = new MeetingsState(
    transport({
      cancelTranscription: async (id) => ({
        ...summary(id, "processing"),
        revision: 60,
      }),
      list: async () => [
        { ...summary("revision-list-new", "cancelled"), revision: 62 },
      ],
    }),
    () => {},
  );
  state.receive({
    ...summary("revision-list-new", "processing"),
    revision: 60,
  });
  await state.cancel("revision-list-new");
  await state.refreshList();
  assert.equal(state.snapshot.active, null);
  assert.equal(state.snapshot.cancellingId, null);
  assert.equal(state.snapshot.meetings[0]?.revision, 62);
}

// An old terminal cancel acknowledgement cannot clear a newer job's pending request.
{
  const acknowledged = deferred<MeetingSummary>();
  let reads = 0;
  const state = new MeetingsState(
    transport({
      cancelTranscription: () => acknowledged.promise,
      transcript: async (id) => {
        reads++;
        return transcript(id);
      },
    }),
    () => {},
  );
  state.receive({
    ...summary("revision-cancel-old", "processing"),
    revision: 72,
  });
  await state.select("revision-cancel-old");
  const cancelling = state.cancel("revision-cancel-old");
  acknowledged.resolve({
    ...summary("revision-cancel-old", "failed"),
    revision: 70,
  });
  await cancelling;
  assert.equal(state.snapshot.active?.revision, 72);
  assert.equal(state.snapshot.cancellingId, "revision-cancel-old");
  assert.equal(
    reads,
    1,
    "a stale command must not reload the selected transcript",
  );
  state.receive({
    ...summary("revision-cancel-old", "cancelled"),
    revision: 73,
  });
}

// Same-revision polling stays idempotent, including after remount and late old events.
{
  const current = {
    ...summary("revision-remount", "processing"),
    revision: 82,
  };
  const api = transport({
    cancelTranscription: async () => current,
    current: async () => current,
    list: async () => [current],
  });
  const first = new MeetingsState(api, () => {});
  first.receive(current);
  await first.cancel(current.id);
  first.dispose();
  const returned = new MeetingsState(api, () => {});
  returned.receive({ ...summary(current.id, "failed"), revision: 80 });
  await returned.load();
  await returned.pollCurrent();
  assert.equal(returned.snapshot.active?.revision, 82);
  assert.equal(returned.snapshot.cancellingId, current.id);
  returned.receive({ ...summary(current.id, "complete"), revision: 83 });
}

// Global idle must survive an older active event from a different meeting ID.
{
  const completeB = { ...summary("activity-b", "complete"), revision: 103 };
  const state = new MeetingsState(
    transport({ current: async () => completeB }),
    () => {},
  );
  state.receive({ ...summary("activity-a", "processing"), revision: 100 });
  state.receive({ ...summary("activity-b", "processing"), revision: 102 });
  state.receive(completeB);
  state.receive({ ...summary("activity-a", "processing"), revision: 100 });
  assert.equal(state.snapshot.active, null);
  await state.loadCurrent();
  assert.equal(state.snapshot.active, null);
  await state.select("activity-a");
  assert.equal(
    state.snapshot.transcript?.id,
    "activity-a",
    "global activity order must not hide history details",
  );
  state.receive({ ...summary("activity-a", "processing"), revision: 104 });
  state.receive(completeB);
  state.receive({ ...summary("activity-a", "processing"), revision: 104 });
  assert.deepEqual(state.snapshot.active, {
    ...summary("activity-a", "processing"),
    revision: 104,
  });
}

// List order cannot revive an old active row behind a newer terminal row.
{
  const olderA = { ...summary("activity-list-a", "processing"), revision: 200 };
  const completeB = {
    ...summary("activity-list-b", "complete"),
    revision: 203,
  };
  let listed = [olderA, completeB];
  const state = new MeetingsState(
    transport({ list: async () => listed }),
    () => {},
  );
  state.receive(completeB);
  await state.refreshList();
  assert.equal(state.snapshot.active, null);
  assert.equal(
    state.snapshot.meetings.find((row) => row.id === olderA.id)?.status,
    "processing",
  );
  listed = [completeB, olderA];
  await state.refreshList();
  assert.equal(state.snapshot.active, null);
}

// A newer terminal snapshot clears any older active owner, including a different ID.
{
  const state = new MeetingsState(transport(), () => {});
  state.receive({
    ...summary("activity-terminal-a", "processing"),
    revision: 300,
  });
  state.receive({ ...summary("activity-terminal-b", "failed"), revision: 303 });
  assert.equal(state.snapshot.active, null);
}

// Activity floors survive remounts, authoritative idle reads and legacy history rows.
{
  const api = transport({ current: async () => null });
  const first = new MeetingsState(api, () => {});
  first.receive({
    ...summary("activity-remount-a", "processing"),
    revision: 400,
  });
  first.receive({
    ...summary("activity-remount-b", "complete"),
    revision: 403,
  });
  first.dispose();
  const returned = new MeetingsState(api, () => {});
  returned.receive({
    ...summary("activity-remount-a", "processing"),
    revision: 400,
  });
  returned.receive(summary("activity-legacy", "processing"));
  assert.equal(returned.snapshot.active, null);
  assert.ok(
    returned.snapshot.meetings.some((row) => row.id === "activity-legacy"),
  );
  returned.receive({
    ...summary("activity-remount-a", "processing"),
    revision: 404,
  });
  await returned.loadCurrent();
  returned.receive({
    ...summary("activity-remount-a", "processing"),
    revision: 404,
  });
  assert.equal(
    returned.snapshot.active,
    null,
    "same-revision old activity cannot revive an authoritative idle result",
  );
  returned.receive({
    ...summary("activity-remount-a", "processing"),
    revision: 405,
  });
  returned.receive({
    ...summary("activity-remount-b", "complete"),
    revision: 403,
  });
  returned.receive({
    ...summary("activity-remount-a", "processing"),
    revision: 405,
  });
  assert.deepEqual(returned.snapshot.active, {
    ...summary("activity-remount-a", "processing"),
    revision: 405,
  });
}

console.log(
  "meetings state: recording lifecycle, retry, selection and event races passed",
);

assert.deepEqual(
  meetingWarningKeys([
    "meeting_attribution_experimental",
    "meeting_attribution_experimental",
  ]),
  ["meetings.warningAttributionExperimental"],
  "Beta attribution limits must remain visible without exposing raw warning text",
);
