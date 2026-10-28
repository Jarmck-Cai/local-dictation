import assert from "node:assert/strict";
import { CollectedTermsState } from "./collectedTermsState";
import type {
  CollectedTermsPage,
  CollectedTermsTransport,
  CollectedTermsQuery,
} from "./collectedTermsApi";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
const row = (liked = false) => ({
  id: 7,
  term: "Rust",
  record_count: 2,
  last_seen: 3,
  liked,
  deleted: false,
});
const page = (liked = false): CollectedTermsPage => ({
  entries: [row(liked)],
  total: 1,
  offset: 0,
  limit: 30,
});
function fixture() {
  const reads: {
    query: CollectedTermsQuery;
    result: ReturnType<typeof deferred<CollectedTermsPage>>;
  }[] = [];
  const writes: {
    id: number;
    value: boolean;
    kind: string;
    result: ReturnType<typeof deferred<void>>;
  }[] = [];
  const api: CollectedTermsTransport = {
    getPage: (query) => {
      const result = deferred<CollectedTermsPage>();
      reads.push({ query, result });
      return result.promise;
    },
    setLiked: (id, value) => {
      const result = deferred<void>();
      writes.push({ id, value, kind: "like", result });
      return result.promise;
    },
    setDeleted: (id, value) => {
      const result = deferred<void>();
      writes.push({ id, value, kind: "delete", result });
      return result.promise;
    },
  };
  const seen: unknown[] = [];
  return {
    state: new CollectedTermsState(api, (next) => seen.push(next)),
    reads,
    writes,
    seen,
  };
}
// Missing generation guards would let the first query overwrite the second.
{
  const { state, reads } = fixture();
  const a = state.load({ query: "old" });
  const b = state.load({ query: "new" });
  assert.equal(reads.length, 2);
  reads[1].result.resolve(page(true));
  await b;
  reads[0].result.resolve(page(false));
  await a;
  assert.equal(state.snapshot.query, "new");
  assert.equal(
    (state.snapshot.page as CollectedTermsPage | null)?.entries[0].liked,
    true,
  );
}
// Failed reads retain an explicit retry path and recover on the next response.
{
  const { state, reads } = fixture();
  const a = state.load();
  reads[0].result.reject(new Error("synthetic offline failure"));
  await a;
  assert.equal(state.snapshot.failed, true);
  const b = state.load();
  reads[1].result.resolve(page());
  await b;
  assert.equal(state.snapshot.failed, false);
}
// An event while a mutation is pending must not launch a stale snapshot read.
{
  const { state, reads, writes } = fixture();
  const stale = state.load();
  const change = state.like(7, true);
  assert.deepEqual(
    { id: writes[0]?.id, value: writes[0]?.value, kind: writes[0]?.kind },
    { id: 7, value: true, kind: "like" },
  );
  await state.load();
  assert.equal(reads.length, 1);
  reads[0].result.resolve(page(false));
  await stale;
  assert.equal(state.snapshot.page, null);
  writes[0].result.resolve();
  await Promise.resolve();
  assert.equal(reads.length, 2);
  reads[1].result.resolve(page(true));
  await change;
  assert.equal(
    (state.snapshot.page as CollectedTermsPage | null)?.entries[0].liked,
    true,
  );
  assert.equal(state.snapshot.busy, false);
}
// Deletion/restoration uses its own command, query changes remain valid mid-write.
{
  const { state, reads, writes } = fixture();
  const change = state.remove(7, false);
  await state.load({ query: "Rust", filter: "deleted", offset: 0 });
  writes[0].result.resolve();
  await Promise.resolve();
  assert.equal(writes[0].kind, "delete");
  assert.equal(writes[0].value, false);
  assert.equal(reads[0].query.filter, "deleted");
  reads[0].result.resolve({ entries: [], total: 0, offset: 0, limit: 30 });
  await change;
  assert.equal(state.snapshot.page?.total, 0);
}
// Empty last pages after deletion move to the last valid page automatically.
{
  const { state, reads } = fixture();
  const load = state.load({ offset: 60 });
  reads[0].result.resolve({ entries: [], total: 45, offset: 60, limit: 30 });
  await Promise.resolve();
  assert.equal(reads[1].query.offset, 30);
  reads[1].result.resolve({
    entries: [row()],
    total: 45,
    offset: 30,
    limit: 30,
  });
  await load;
  assert.equal(state.snapshot.offset, 30);
}
// A failed write restores controls and an authoritative retry, without false tags.
{
  const { state, reads, writes } = fixture();
  const write = state.like(7, true);
  writes[0].result.reject(new Error("synthetic rejected write"));
  await Promise.resolve();
  reads[0].result.resolve(page(false));
  await write;
  assert.equal(state.snapshot.busy, false);
  assert.equal(state.snapshot.changeFailed, true);
  assert.equal(state.snapshot.page?.entries[0].liked, false);
}
// Unmount invalidates both read completion and delayed mutation refresh.
{
  const { state, reads, seen } = fixture();
  const a = state.load();
  state.dispose();
  const count = seen.length;
  reads[0].result.resolve(page());
  await a;
  assert.equal(seen.length, count);
}
console.log("collected terms state: 7 asynchronous scenarios passed");
