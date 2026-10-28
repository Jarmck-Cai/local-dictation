import assert from "node:assert/strict";
import type { HistoryEntry } from "../bindings";
import type {
  HistorySearchPage,
  HistorySearchRequest,
} from "./historySearchApi";
import { HistorySearchState, recentHistoryEntries } from "./historySearchState";
import { summarizeUsage } from "./usageSummary";

const row = (id: number, text = `synthetic-${id}`): HistoryEntry => ({
  id,
  timestamp: 1,
  file_name: "synthetic.wav",
  title: "",
  saved: false,
  transcription_text: text,
  post_processed_text: null,
  post_process_prompt: null,
  post_process_requested: false,
});
let source = Array.from({ length: 350 }, (_, i) => row(350 - i));
source[349].post_processed_text = "终稿专词";
assert.deepEqual(
  recentHistoryEntries(source).map((entry) => entry.id),
  [350, 349, 348, 347, 346, 345, 344, 343, 342, 341, 340, 339, 338, 337, 336],
);
const fetchPage = async (
  request: HistorySearchRequest,
): Promise<HistorySearchPage> => {
  const anchor = request.anchor_id ?? source[0]?.id ?? null;
  const matching = source.filter(
    (entry) =>
      anchor !== null &&
      entry.id <= anchor &&
      (request.cursor === null || entry.id < request.cursor) &&
      (!request.query ||
        entry.transcription_text.includes(request.query) ||
        entry.post_processed_text?.includes(request.query)),
  );
  const entries = matching.slice(0, request.limit);
  const more = matching.length > request.limit;
  return {
    entries,
    has_more: more,
    anchor_id: anchor,
    next_cursor: more ? entries[entries.length - 1].id : null,
  };
};
const ids = (state: HistorySearchState) =>
  state.snapshot.entries.map((entry) => entry.id);
const workspace = new HistorySearchState(fetchPage);
await workspace.setActive(true);
assert.equal(workspace.snapshot.entries.length, 30);
assert.equal(workspace.snapshot.anchorId, 350);
workspace.setDraftText(350, "尚未提交的纠正");
workspace.toggleExpanded(350);
await workspace.next();
assert.equal(workspace.snapshot.page, 2);
assert.equal(ids(workspace)[0], 320);
source.unshift(row(351));
workspace.receive({ action: "added", entry: row(351) });
assert.equal(workspace.snapshot.refreshAvailable, true);
await workspace.previous();
assert.equal(
  ids(workspace)[0],
  350,
  "new arrivals cannot shift an anchored page",
);
assert.equal(workspace.snapshot.drafts.get(350)?.text, "尚未提交的纠正");
assert.equal(workspace.snapshot.expandedId, 350);
await workspace.setActive(false);
await workspace.setActive(true);
assert.equal(workspace.snapshot.drafts.get(350)?.text, "尚未提交的纠正");
await workspace.refresh();
assert.equal(ids(workspace)[0], 351);
assert.equal(workspace.snapshot.refreshAvailable, false);
await workspace.setInput("终稿专词");
assert.deepEqual(
  ids(workspace),
  [1],
  "search reaches final text beyond 300 prefetched rows",
);
await workspace.setInput("synthetic-1");
assert.ok(ids(workspace).includes(199), "raw text is searchable");
const beforeComposition = workspace.snapshot.query;
workspace.compositionStart();
await workspace.setInput("半成品");
assert.equal(workspace.snapshot.query, beforeComposition);
await workspace.compositionEnd("终稿专词");
assert.deepEqual(ids(workspace), [1]);
workspace.compositionStart();
await workspace.setInput("未完成");
await workspace.setActive(false);
await workspace.setActive(true);
assert.equal(
  workspace.snapshot.composing,
  false,
  "leaving an IME field cannot leave search stuck composing",
);
assert.equal(
  workspace.snapshot.input,
  "终稿专词",
  "leaving during composition keeps the last committed query",
);
await workspace.setInput("😀".repeat(257));
assert.equal(workspace.snapshot.invalidQuery, true);
assert.deepEqual(ids(workspace), []);
await workspace.setInput(" \n");
assert.equal(workspace.snapshot.invalidQuery, false);
assert.equal(workspace.snapshot.entries.length, 30);

type Pending = {
  request: HistorySearchRequest;
  resolve: (page: HistorySearchPage) => void;
  reject: (error: Error) => void;
};
const pending: Pending[] = [];
const racing = new HistorySearchState(
  (request) =>
    new Promise((resolve, reject) =>
      pending.push({ request, resolve, reject }),
    ),
);
const initial = racing.setActive(true);
const latest = racing.setInput("终稿专词");
pending[1].resolve(await fetchPage(pending[1].request));
await latest;
pending[0].resolve(await fetchPage(pending[0].request));
await initial;
assert.deepEqual(
  ids(racing),
  [1],
  "late old query must not replace new results",
);

racing.setDraftText(1, "草稿");
racing.toggleExpanded(1);
const retry = racing.refresh();
racing.receive({ action: "deleted", id: 1 });
pending[2].resolve({
  entries: [row(1)],
  has_more: false,
  next_cursor: null,
  anchor_id: 351,
});
await retry;
assert.deepEqual(
  ids(racing),
  [],
  "deletion cannot be undone by a late response",
);
assert.equal(racing.snapshot.drafts.has(1), false);
assert.equal(racing.snapshot.expandedId, null);
pending[3].resolve({
  entries: [],
  has_more: false,
  next_cursor: null,
  anchor_id: 351,
});
await new Promise((resolve) => setTimeout(resolve, 0));

const failing = new HistorySearchState(async (request) => {
  if (failure) throw new Error("synthetic failure");
  return fetchPage(request);
});
let failure = true;
await failing.setActive(true);
assert.equal(failing.snapshot.failed, true);
assert.deepEqual(ids(failing), []);
failure = false;
await failing.retry();
assert.equal(failing.snapshot.failed, false);
assert.equal(failing.snapshot.entries.length, 30);

// A deleted last-page row rolls back through the cursor stack without a dead end.
source = [row(31), row(30)];
const emptyPage = new HistorySearchState(async (request) =>
  fetchPage({ ...request, limit: 1 }),
);
await emptyPage.setActive(true);
await emptyPage.next();
source = [row(31)];
emptyPage.receive({ action: "deleted", id: 30 });
await new Promise((resolve) => setTimeout(resolve, 0));
assert.equal(emptyPage.snapshot.page, 1);
assert.deepEqual(ids(emptyPage), [31]);

// An update invalidates search membership, without discarding an edited draft.
emptyPage.setDraftText(31, "edited while searching");
emptyPage.setSubmitting(31, true);
await emptyPage.setActive(false);
await emptyPage.setActive(true);
assert.equal(
  emptyPage.snapshot.drafts.get(31)?.submitting,
  true,
  "submission in flight stays busy across remounts",
);
emptyPage.setDraftText(31, "edited while searching");
assert.equal(emptyPage.snapshot.drafts.get(31)?.submitting, true);
source = [row(31, "changed")];
emptyPage.receive({ action: "updated", entry: source[0] });
await new Promise((resolve) => setTimeout(resolve, 0));
assert.equal(emptyPage.snapshot.entries[0].transcription_text, "changed");
assert.equal(emptyPage.snapshot.drafts.get(31)?.text, "edited while searching");
emptyPage.markSubmitted(31, "earlier submission");
emptyPage.setSubmitting(31, false);
assert.equal(
  emptyPage.snapshot.drafts.get(31)?.text,
  "edited while searching",
  "late submission ack cannot overwrite a newer draft",
);
const review = {
  candidates: [
    {
      id: 91,
      term: "SyntheticTerm",
      replacement_from: null,
      context: null,
      source: "correction",
      occurrences: 1,
    },
  ],
  entries: [],
};
emptyPage.markSubmitted(31, "submitted sample", review);
await emptyPage.refresh();
assert.equal(
  emptyPage.snapshot.drafts.get(31)?.review?.candidates[0].id,
  91,
  "submitted candidates survive list remounts",
);
emptyPage.setReview(31, { candidates: [], entries: [] });
await emptyPage.setActive(false);
await emptyPage.setActive(true);
assert.equal(
  emptyPage.snapshot.drafts.get(31)?.review?.candidates.length,
  0,
  "candidate review changes survive remounts",
);
let resolveReview!: (value: typeof review) => void;
const staleReview = emptyPage.refreshReview(
  31,
  () =>
    new Promise((resolve) => {
      resolveReview = resolve;
    }),
);
emptyPage.setReview(31, { candidates: [], entries: [] });
resolveReview(review);
await staleReview;
assert.equal(
  emptyPage.snapshot.drafts.get(31)?.review?.candidates.length,
  0,
  "late refresh cannot undo a candidate confirmation",
);
await emptyPage.refreshReview(31, async () => review);
assert.equal(
  emptyPage.snapshot.drafts.get(31)?.review?.candidates[0].id,
  91,
  "a remounted form can reconcile a submission or review completed while absent",
);

const shared = [row(5), row(4)];
const beforeUsage = summarizeUsage(shared, [], 2000).records;
await workspace.setInput("changed");
assert.deepEqual(
  summarizeUsage(shared, [], 2000).records,
  beforeUsage,
  "search leaves the shared usage source alone",
);
const disposed = racing.refresh();
racing.dispose();
pending[pending.length - 1].resolve(
  await fetchPage(pending[pending.length - 1].request),
);
await disposed;
assert.deepEqual(ids(racing), []);
console.log(
  "history search state: anchored pages, IME, stale/deleted responses, drafts, retries and usage isolation passed",
);
