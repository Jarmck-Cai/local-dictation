import assert from "node:assert/strict";
import {
  SnapshotGuard,
  mergeTiming,
  cloudState,
  distinctCleanupReason,
} from "./dictationState";
import type { DictationTiming } from "./dictationApi";

const requests = new SnapshotGuard();
const first = requests.begin();
const retry = requests.begin();
assert.equal(
  requests.isCurrent(first),
  false,
  "a retry supersedes its old snapshot",
);
assert.equal(requests.isCurrent(retry), true);
requests.invalidate();
assert.equal(
  requests.isCurrent(retry),
  false,
  "live events supersede in-flight snapshots",
);
const final = requests.begin();
requests.dispose();
assert.equal(
  requests.isCurrent(final),
  false,
  "unmounted consumers cannot accept responses",
);
assert.equal(requests.isAlive(), false);

const timing = (
  id: string,
  timestamp: number,
  paste: number | null,
): DictationTiming => ({
  id,
  recorded_at_ms: timestamp,
  history_id: 1,
  asr_ms: 300,
  cleanup_ms: 1200,
  paste_ms: paste,
  total_ms: 1600,
  cleanup_budget_ms: 3000,
  mode: "claude",
  outcome: "ready",
});
const progress = timing("same", 100, null);
const complete = timing("same", 100, 30);
const merged = mergeTiming([timing("new", 200, 20), progress], complete);
assert.deepEqual(
  merged.map((item) => item.id),
  ["new", "same"],
);
assert.equal(
  merged[1].paste_ms,
  30,
  "a completion replaces its in-progress measurement",
);
assert.equal(mergeTiming([complete], timing("old", 1, 5), 1)[0].id, "same");

assert.equal(
  cloudState(
    {
      manual_paused: true,
      cooldown_remaining_ms: 100,
      consecutive_failures: 3,
    },
    1000,
    5000,
  ),
  "manual",
);
assert.equal(
  cloudState(
    {
      manual_paused: false,
      cooldown_remaining_ms: 10000,
      consecutive_failures: 3,
    },
    1000,
    5000,
  ),
  "cooldown",
);
assert.equal(
  cloudState(
    {
      manual_paused: false,
      cooldown_remaining_ms: 10000,
      consecutive_failures: 3,
    },
    1000,
    11000,
  ),
  "nextRecording",
);
assert.equal(
  cloudState(
    { manual_paused: false, cooldown_remaining_ms: 0, consecutive_failures: 0 },
    1000,
    1000,
  ),
  "active",
);
assert.equal(
  cloudState(
    { manual_paused: false, cooldown_remaining_ms: 0, consecutive_failures: 3 },
    600000,
    600000,
  ),
  "nextRecording",
  "a first snapshot after cooldown expiry still awaits the next recording and offers resume",
);
assert.equal(
  cloudState(
    { manual_paused: false, cooldown_remaining_ms: 0, consecutive_failures: 2 },
    600000,
    600000,
  ),
  "active",
  "failures below the cooldown threshold do not pause the cloud",
);
const timeoutAndPasteFailure: DictationTiming = {
  ...complete,
  outcome: "paste_failed",
  cleanup_reason: "cleanup_timeout",
};
assert.equal(timeoutAndPasteFailure.outcome, "paste_failed");
assert.equal(
  distinctCleanupReason(timeoutAndPasteFailure),
  "cleanup_timeout",
  "an insertion failure does not hide an earlier cleanup timeout",
);
assert.equal(
  distinctCleanupReason({ ...timeoutAndPasteFailure, outcome: "cancelled" }),
  "cleanup_timeout",
  "a later cancellation can retain its independent cleanup reason",
);
assert.equal(
  distinctCleanupReason({
    ...timeoutAndPasteFailure,
    outcome: "cleanup_timeout",
  }),
  null,
  "the same cleanup reason is not displayed twice",
);
assert.equal(
  distinctCleanupReason(complete),
  null,
  "old records without cleanup_reason remain readable",
);
assert.equal(
  distinctCleanupReason({ ...complete, cleanup_reason: null }),
  null,
);
assert.equal(
  distinctCleanupReason({ ...complete, cleanup_reason: "  " }),
  null,
);
console.log(
  "dictation state: async response, timing reconciliation, and cooldown assertions passed",
);
