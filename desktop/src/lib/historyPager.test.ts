import assert from "node:assert/strict";
import type { HistoryEntry, PaginatedHistory } from "../bindings";
import { HistoryPager, historyUsageStatus } from "./historyPager";
import { summarizeUsage } from "./usageSummary";

const now = Date.now();
const entry = (id: number, text = String(id)): HistoryEntry => ({
  id,
  timestamp: now / 1000,
  transcription_text: text,
  post_processed_text: null,
  file_name: "synthetic.wav",
  saved: false,
  title: "",
  post_process_prompt: null,
  post_process_requested: false,
});
const source = Array.from({ length: 405 }, (_, i) => entry(500 - i));
const pageAt = (cursor: number | null, limit: number): PaginatedHistory => {
  const matching = source.filter((row) => cursor === null || row.id < cursor);
  return {
    entries: matching.slice(0, limit),
    has_more: matching.length > limit,
  };
};
const calls: { cursor: number | null; limit: number }[] = [];
const pager = new HistoryPager(async (cursor, limit) => {
  calls.push({ cursor, limit });
  return pageAt(cursor, limit);
});
await pager.prefetch();
assert.equal(pager.snapshot.entries.length, 300);
assert.equal(
  calls.length,
  4,
  "normal home browsing prefetches only a bounded 300, not the entire history",
);
assert.equal(pager.snapshot.hasMore, true);
await pager.loadMore();
assert.equal(
  pager.snapshot.entries.length,
  330,
  "the 301st retained record remains accessible through a real backend page",
);
assert.equal(pager.snapshot.entries[300].id, 200);
assert.equal(calls[4].cursor, 201);
assert.equal(
  summarizeUsage(pager.snapshot.entries, [], now).records.length,
  300,
  "loading older history does not expand the statistics scope",
);

type Deferred = {
  resolve: (page: PaginatedHistory) => void;
  reject: (error: Error) => void;
};
let pending: Deferred | undefined;
let delay = false;
const racingCalls: { cursor: number | null; limit: number }[] = [];
const racing = new HistoryPager((cursor, limit) => {
  racingCalls.push({ cursor, limit });
  return delay
    ? new Promise<PaginatedHistory>((resolve, reject) => {
        pending = { resolve, reject };
      })
    : Promise.resolve(pageAt(cursor, limit));
});
await racing.prefetch();
delay = true;
const firstMore = racing.loadMore();
const duplicateMore = racing.loadMore();
assert.equal(
  racingCalls.length,
  5,
  "overlapping load-more gestures share one backend request",
);
racing.receive({ action: "added", entry: entry(999, "new") });
racing.receive({ action: "updated", entry: entry(999, "newer") });
racing.receive({ action: "updated", entry: entry(250, "updated existing") });
racing.receive({ action: "updated", entry: entry(200, "updated unloaded") });
racing.receive({ action: "deleted", id: 199 });
pending!.resolve({
  entries: [entry(250, "stale duplicate"), ...pageAt(201, 30).entries],
  has_more: true,
});
await Promise.all([firstMore, duplicateMore]);
assert.equal(
  racing.snapshot.entries.find((row) => row.id === 999)?.transcription_text,
  "newer",
);
assert.equal(
  racing.snapshot.entries.find((row) => row.id === 250)?.transcription_text,
  "updated existing",
);
assert.equal(
  racing.snapshot.entries.find((row) => row.id === 200)?.transcription_text,
  "updated unloaded",
);
assert.equal(
  racing.snapshot.entries.some((row) => row.id === 199),
  false,
  "a late page cannot revive a deleted record",
);
assert.equal(
  new Set(racing.snapshot.entries.map((row) => row.id)).size,
  racing.snapshot.entries.length,
);

const failedCursor = racingCalls[racingCalls.length - 1].cursor;
const failure = racing.loadMore();
const retryCursor = racingCalls[racingCalls.length - 1].cursor;
assert.notEqual(
  retryCursor,
  failedCursor,
  "the accepted page advances using its raw page cursor",
);
racing.receive({ action: "toggled", id: 250 });
racing.receive({ action: "toggled", id: 161 });
racing.receive({
  action: "updated",
  entry: entry(162, "updated before retry"),
});
racing.receive({ action: "toggled", id: 162 });
racing.receive({ action: "deleted", id: 160 });
pending!.reject(new Error("synthetic page failure"));
await failure;
assert.equal(racing.snapshot.moreFailed, true);
assert.equal(
  historyUsageStatus(racing.snapshot).usageFailed,
  false,
  "a page failure beyond a complete 300-record usage window does not invalidate usage",
);
assert.equal(
  racing.snapshot.failed,
  false,
  "a later page failure does not invalidate the already loaded history",
);
assert.equal(
  racing.snapshot.entries.find((row) => row.id === 250)?.saved,
  true,
);
racing.receive({ action: "toggled", id: 163 });
const retry = racing.loadMore();
assert.equal(
  racingCalls[racingCalls.length - 1].cursor,
  retryCursor,
  "failure cannot skip or reset the backend cursor",
);
pending!.resolve({
  entries: [
    entry(250),
    ...pageAt(retryCursor, 30).entries.map((row) =>
      [161, 162, 163].includes(row.id) ? { ...row, saved: true } : row,
    ),
  ],
  has_more: true,
});
await retry;
assert.equal(racing.snapshot.moreFailed, false);
assert.equal(
  racing.snapshot.entries.find((row) => row.id === 161)?.saved,
  true,
  "a toggle before the retry snapshot is not replayed on a newly loaded row",
);
assert.equal(
  racing.snapshot.entries.find((row) => row.id === 162)?.saved,
  true,
);
assert.equal(
  racing.snapshot.entries.find((row) => row.id === 162)?.transcription_text,
  "updated before retry",
  "an absolute unloaded update retains the effect of a later toggle across retry",
);
assert.equal(
  racing.snapshot.entries.find((row) => row.id === 163)?.saved,
  true,
  "a relative event while waiting to retry also predates the new snapshot",
);
assert.equal(
  racing.snapshot.entries.find((row) => row.id === 250)?.saved,
  true,
  "retry does not replay a live toggle on existing rows",
);
assert.equal(
  racing.snapshot.entries.some((row) => row.id === 160),
  false,
);

const backfillCalls: { cursor: number | null; limit: number }[] = [];
const backfill = new HistoryPager(async (cursor, limit) => {
  backfillCalls.push({ cursor, limit });
  return pageAt(cursor, limit);
});
await backfill.prefetch();
backfill.receive({ action: "deleted", id: 500 });
await backfill.loadMore(); // Joins the automatic deletion backfill already in progress.
assert.equal(
  backfill.snapshot.entries.length,
  300,
  "deleting within the initial 300 automatically fills the gap from the server",
);
assert.equal(backfill.snapshot.entries[299].id, 200);
assert.equal(
  backfillCalls[4].limit,
  1,
  "automatic backfill fetches the missing row rather than all older history",
);
assert.equal(
  summarizeUsage(backfill.snapshot.entries, [], now).records.length,
  300,
);
backfill.receive({ action: "deleted", id: 500 });
assert.equal(
  backfillCalls.length,
  5,
  "the explicit-delete acknowledgement cannot trigger a duplicate backfill",
);

let failBackfill = false;
const incomplete = new HistoryPager(async (cursor, limit) => {
  if (failBackfill) throw new Error("synthetic backfill failure");
  return pageAt(cursor, limit);
});
await incomplete.prefetch();
failBackfill = true;
incomplete.receive({ action: "deleted", id: 500 });
assert.equal(
  historyUsageStatus(incomplete.snapshot).usageLoading,
  true,
  "usage is incomplete while its deletion gap is being filled",
);
await incomplete.loadMore();
assert.equal(incomplete.snapshot.entries.length, 299);
assert.equal(
  historyUsageStatus(incomplete.snapshot).usageFailed,
  true,
  "a failed deletion backfill cannot be presented as a complete 299-record usage window",
);
failBackfill = false;
await incomplete.prefetch();
assert.equal(incomplete.snapshot.entries.length, 300);
assert.deepEqual(historyUsageStatus(incomplete.snapshot), {
  usageLoading: false,
  usageFailed: false,
});

let initialAttempt = 0;
const initialCalls: (number | null)[] = [];
const initialRetry = new HistoryPager(async (cursor, limit) => {
  initialCalls.push(cursor);
  initialAttempt += 1;
  if (initialAttempt === 2)
    throw new Error("synthetic initial prefetch failure");
  return pageAt(cursor, limit);
});
await initialRetry.prefetch();
assert.equal(initialRetry.snapshot.entries.length, 30);
assert.equal(initialRetry.snapshot.failed, true);
await initialRetry.prefetch();
assert.equal(
  initialCalls[2],
  initialCalls[1],
  "initial prefetch retry also resumes at the failed cursor",
);
assert.equal(initialRetry.snapshot.entries.length, 300);

const disposePending = racing.loadMore();
const beforeDispose = racing.snapshot.entries;
racing.dispose();
pending!.resolve(pageAt(1, 30));
await disposePending;
assert.equal(
  racing.snapshot.entries,
  beforeDispose,
  "a disposed controller ignores a late backend page",
);
console.log(
  "history pager: 301st record, bounded prefetch, event races, retry cursor, deletion backfill and disposal passed",
);
