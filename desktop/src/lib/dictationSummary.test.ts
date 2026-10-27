import assert from "node:assert/strict";
import type { DictationTiming } from "./dictationApi";
import { summarizeResources, summarizeTimings } from "./dictationSummary";

const now = 8 * 24 * 60 * 60 * 1000;
const timing = (
  id: string,
  total: number,
  changes: Partial<DictationTiming> = {},
): DictationTiming => ({
  id,
  history_id: null,
  recorded_at_ms: now,
  asr_ms: total / 4,
  cleanup_ms: total / 2,
  paste_ms: total / 10,
  total_ms: total,
  cleanup_budget_ms: 3000,
  mode: "claude",
  outcome: "cleaned",
  ...changes,
});

const summary = summarizeTimings(
  [
    timing("fast", 1000),
    timing("slow", 3000),
    timing("cancel", 99999, { outcome: "cancelled", paste_ms: null }),
    timing("silence", 99999, { outcome: "no_audio", paste_ms: null }),
    timing("old", 99999, { recorded_at_ms: now - 7 * 24 * 60 * 60 * 1000 }),
    timing("missing-paste", 99999, { paste_ms: null }),
  ],
  now,
);
assert.equal(
  summary.records.length,
  5,
  "expired records cannot affect any summary",
);
assert.deepEqual(
  summary.medians,
  { asr: 500, cleanup: 1000, paste: 200, total: 2000 },
  "cancellation, silence and unconfirmed insertion cannot inflate successful wait",
);
assert.equal(summary.completed.length, 2);
assert.equal(summary.cancelled, 1);
assert.equal(summary.noAudio, 1);
assert.equal(
  summary.unconfirmed,
  1,
  "a cleanup outcome alone does not confirm insertion",
);
assert.equal(summary.smallSample, true);
assert.deepEqual(
  summary.slowest.map((row) => row.id),
  ["slow", "fast"],
);

const fallback = summarizeTimings(
  [
    timing("timeout", 2000, {
      outcome: "cleanup_timeout",
      cleanup_reason: "cleanup_timeout",
    }),
    timing("failed-paste", 9000, {
      outcome: "paste_failed",
      cleanup_reason: "cleanup_timeout",
      paste_ms: null,
    }),
    timing("local", 1200, {
      mode: "local",
      outcome: "cleanup_request_failed",
      cleanup_reason: "cleanup_request_failed",
    }),
    timing("old-cloud", 4000, { outcome: "cloud_paused" }),
  ],
  now,
);
assert.deepEqual(
  fallback.fallbackReasons,
  [
    { reason: "cleanup_timeout", count: 2 },
    { reason: "cloud_paused", count: 1 },
  ],
  "cloud fallback is counted independently of insertion, without local-mode failures",
);
assert.equal(fallback.completed.length, 3);
assert.equal(fallback.unconfirmed, 1);

const bounded = summarizeTimings(
  Array.from({ length: 305 }, (_, index) =>
    timing(String(index), index + 1, { recorded_at_ms: now - index }),
  ),
  now,
);
assert.equal(
  bounded.records.length,
  300,
  "all statistics follow the existing 300 record limit",
);
assert.equal(bounded.smallSample, false);
assert.deepEqual(summarizeTimings([], now).medians, null);
const malformed = summarizeTimings(
  [timing("invalid", Number.NaN), timing("negative", -100)],
  now,
);
assert.equal(
  malformed.completed.length,
  0,
  "invalid durations do not become success samples",
);
assert.equal(malformed.medians, null);

const resources = {
  ...timing("resource", 1000),
  resource_snapshots: [
    { stage: "started", working_set_bytes: 1024, process_cpu_ms: 500 },
    { stage: "asr", working_set_bytes: 4096, process_cpu_ms: 750 },
    { stage: "finished", working_set_bytes: 2048, process_cpu_ms: 900 },
  ],
};
assert.deepEqual(
  summarizeResources(resources),
  { sampledWorkingSetBytes: 4096, processCpuMs: 400, samples: 3 },
  "working set is the boundary maximum while CPU is a process-time difference",
);
assert.deepEqual(
  summarizeResources(timing("legacy", 1000)),
  { sampledWorkingSetBytes: null, processCpuMs: null, samples: 0 },
  "legacy rows have no invented resource measurements",
);
assert.equal(
  summarizeResources({
    ...resources,
    resource_snapshots: [
      { stage: "started", working_set_bytes: null, process_cpu_ms: null },
      resources.resource_snapshots[2],
    ],
  }).processCpuMs,
  null,
  "missing initial CPU readings cannot be replaced with process lifetime CPU",
);
assert.equal(
  summarizeResources({
    ...resources,
    resource_snapshots: [
      resources.resource_snapshots[2],
      resources.resource_snapshots[0],
    ],
  }).processCpuMs,
  null,
  "counter rollback cannot become a negative CPU duration",
);
console.log(
  "dictation summary: retention, medians, completion, fallback and sample boundaries passed",
);
