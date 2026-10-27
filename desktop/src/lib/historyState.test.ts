import assert from "node:assert/strict";
import type { HistoryEntry } from "../bindings";
import { mergeHistorySnapshot, applyHistoryEvent } from "./historyState";

const entry = (id: number, text = String(id)): HistoryEntry => ({
  id,
  transcription_text: text,
  timestamp: id,
  post_processed_text: null,
  file_name: "synthetic.wav",
  saved: false,
  title: "",
  post_process_prompt: null,
  post_process_requested: false,
});
const changes = [
  { action: "added" as const, entry: entry(4) },
  { action: "updated" as const, entry: entry(3, "new text") },
  { action: "deleted" as const, id: 2 },
];
const initial = mergeHistorySnapshot(
  [],
  [entry(3), entry(2), entry(1)],
  changes,
  true,
);
assert.deepEqual(
  initial.map((row) => row.id),
  [4, 3, 1],
  "an added event does not throw away the older first-page records",
);
assert.equal(
  initial[1].transcription_text,
  "new text",
  "an in-flight old snapshot cannot overwrite a newer event",
);
const page = mergeHistorySnapshot(
  initial,
  [entry(2), entry(1), entry(0)],
  [{ action: "deleted", id: 2 }],
  false,
);
assert.deepEqual(
  page.map((row) => row.id),
  [4, 3, 1, 0],
  "pagination deduplicates and cannot resurrect an in-flight deletion",
);
assert.equal(
  applyHistoryEvent(initial, { action: "updated", entry: entry(9) }).length,
  3,
  "an unrelated update cannot insert an unloaded row",
);
assert.equal(
  applyHistoryEvent(initial, { action: "toggled", id: 3 })[1].saved,
  true,
);
assert.equal(
  mergeHistorySnapshot(initial, [entry(1)], [], true).length,
  1,
  "an explicit refresh accepts a fresh snapshot when no events raced",
);
const toggled = applyHistoryEvent([entry(1)], { action: "toggled", id: 1 });
assert.equal(
  mergeHistorySnapshot(
    toggled,
    [entry(1)],
    [{ action: "toggled", id: 1 }],
    false,
  )[0].saved,
  true,
  "a toggled event already applied to current rows is not applied twice during pagination",
);
assert.equal(
  mergeHistorySnapshot(
    [entry(4, "newer")],
    [entry(3)],
    [
      { action: "added", entry: entry(4, "earlier") },
      { action: "updated", entry: entry(4, "newer") },
    ],
    false,
  )[0].transcription_text,
  "newer",
  "a page racing add then update cannot restore the earlier added payload",
);
console.log(
  "history state: first snapshot, event overlay, deletion and pagination assertions passed",
);
