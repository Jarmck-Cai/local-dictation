import assert from "node:assert/strict";
import type { HistoryEntry } from "../bindings";
import type { DictationTiming } from "./dictationApi";
import { summarizeUsage } from "./usageSummary";

const now = Date.UTC(2026, 9, 27, 12);
const day = 24 * 60 * 60 * 1000;
const entry = (
  id: number,
  text: string,
  changes: Partial<HistoryEntry> = {},
): HistoryEntry => ({
  id,
  timestamp: now / 1000,
  transcription_text: text,
  post_processed_text: null,
  file_name: "synthetic.wav",
  saved: false,
  title: "",
  post_process_prompt: null,
  post_process_requested: false,
  ...changes,
});
const timing = (
  id: number,
  changes: Partial<DictationTiming> = {},
): DictationTiming => ({
  id: String(id),
  history_id: id,
  recorded_at_ms: now,
  recording_ms: 1000,
  asr_ms: 100,
  cleanup_ms: 100,
  paste_ms: 100,
  total_ms: 1000,
  cleanup_budget_ms: 3000,
  mode: "claude",
  outcome: "cleaned",
  ...changes,
});

const mixed = summarizeUsage(
  [entry(1, "你好， Hello world! 𠀀🙂\n")],
  [timing(1)],
  now,
);
assert.equal(mixed.transcriptions, 1);
assert.equal(
  mixed.characters,
  16,
  "Unicode code points count once; whitespace does not count",
);
assert.equal(mixed.estimatedSamples, 1);
assert.equal(
  mixed.savedMs,
  4000,
  "3 Han at 60/min + 2 words at 40/min, minus recording and processing",
);
assert.equal(
  summarizeUsage(
    [entry(1, "original", { post_processed_text: "新稿" })],
    [],
    now,
  ).characters,
  2,
);
assert.equal(
  summarizeUsage(
    [entry(1, "original", { post_processed_text: " \n" })],
    [],
    now,
  ).characters,
  8,
);

const absent = [undefined, null, NaN, Infinity, -1, "1000"];
for (const value of absent) {
  const result = summarizeUsage(
    [entry(1, "你好")],
    [timing(1, { recording_ms: value as number })],
    now,
  );
  assert.equal(
    result.estimatedSamples,
    0,
    `unknown or invalid recording time ${String(value)} cannot become zero`,
  );
}
assert.equal(
  summarizeUsage([entry(1, "你好")], [timing(1, { total_ms: NaN })], now)
    .estimatedSamples,
  0,
);
assert.equal(
  summarizeUsage([entry(1, "你好")], [timing(1, { paste_ms: null })], now)
    .estimatedSamples,
  0,
);
assert.equal(summarizeUsage([entry(1, "你好")], [], now).estimatedSamples, 0);
for (const outcome of [
  "cancelled",
  "asr_failed",
  "paste_failed",
  "paste_unconfirmed",
  "paste_skipped",
  "empty",
  "no_audio",
  "pending",
]) {
  assert.equal(
    summarizeUsage([entry(1, "你好")], [timing(1, { outcome })], now)
      .estimatedSamples,
    0,
  );
}
assert.equal(
  summarizeUsage([entry(1, "\n \t")], [timing(1)], now).transcriptions,
  0,
);
assert.equal(
  summarizeUsage([entry(1, "\n \t")], [timing(1)], now).estimatedSamples,
  0,
);
assert.equal(
  summarizeUsage([entry(1, "一")], [timing(1)], now).savedMs,
  0,
  "each record floors savings at zero",
);

const scope = summarizeUsage(
  [
    entry(1, "a"),
    entry(1, "duplicated"),
    entry(2, "expired", { timestamp: (now - 7 * day) / 1000 }),
    entry(3, "future", { timestamp: (now + 1) / 1000 }),
    entry(4, "bad", { timestamp: NaN }),
  ],
  [timing(1), timing(1)],
  now,
);
assert.equal(
  scope.transcriptions,
  1,
  "duplicate, expired, future and invalid entries do not inflate usage",
);
assert.equal(scope.characters, 1);
assert.equal(
  scope.estimatedSamples,
  1,
  "duplicate metrics cannot double count savings",
);
assert.equal(
  summarizeUsage(
    [entry(1, "你好")],
    [timing(1, { recorded_at_ms: now - 8 * day })],
    now,
  ).estimatedSamples,
  0,
);
const recent = Array.from({ length: 305 }, (_, i) =>
  entry(i + 1, "一", { timestamp: (now - i) / 1000 }),
);
assert.equal(summarizeUsage(recent, [], now).transcriptions, 300);
assert.equal(
  summarizeUsage(recent.reverse(), [], now).records[0].id,
  1,
  "cap applies after sorting newest first",
);
console.log(
  "usage summary: mixed Unicode, timing eligibility, deduplication and retention assertions passed",
);
