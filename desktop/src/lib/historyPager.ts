import type {
  HistoryEntry,
  HistoryUpdatePayload,
  PaginatedHistory,
} from "../bindings";
import { applyHistoryEvent, mergeHistorySnapshot } from "./historyState";
import { USAGE_MAX_RECORDS } from "./usageSummary";

export interface HistoryPagerSnapshot {
  entries: HistoryEntry[];
  loading: boolean;
  failed: boolean;
  hasMore: boolean;
  loadingMore: boolean;
  moreFailed: boolean;
}

export function historyUsageStatus(snapshot: HistoryPagerSnapshot) {
  const incomplete =
    snapshot.entries.length < USAGE_MAX_RECORDS && snapshot.hasMore;
  return {
    usageLoading: snapshot.loading || (incomplete && snapshot.loadingMore),
    usageFailed: snapshot.failed || (incomplete && snapshot.moreFailed),
  };
}

/** The 300-row usage window is a prefetch target, never a history list limit. */
export class HistoryPager {
  snapshot: HistoryPagerSnapshot = {
    entries: [],
    loading: false,
    failed: false,
    hasMore: true,
    loadingMore: false,
    moreFailed: false,
  };
  private cursor: number | null = null;
  private acceptedPage = false;
  private operation: Promise<void> | null = null;
  private reading = false;
  private changes: HistoryUpdatePayload[] = [];
  private disposed = false;

  constructor(
    private fetchPage: (
      cursor: number | null,
      limit: number,
    ) => Promise<PaginatedHistory>,
    private onChange: (snapshot: HistoryPagerSnapshot) => void = () => {},
  ) {}

  private publish(change: Partial<HistoryPagerSnapshot>) {
    if (this.disposed) return;
    this.snapshot = { ...this.snapshot, ...change };
    this.onChange(this.snapshot);
  }

  private async readPage(limit: number): Promise<void> {
    this.reading = true;
    try {
      const page = await this.fetchPage(this.cursor, limit);
      if (this.disposed) return;
      const next = page.entries[page.entries.length - 1]?.id;
      const overlay = this.changes;
      this.changes = [];
      const entries = mergeHistorySnapshot(
        this.snapshot.entries,
        page.entries,
        overlay,
        !this.acceptedPage,
      );
      const hasMore =
        page.has_more && next !== undefined && next !== this.cursor;
      // Advance only after accepting a successful raw page. Live additions or
      // deletions must not move this cursor, and a failed request leaves it intact.
      this.cursor = next ?? this.cursor;
      this.acceptedPage = true;
      this.publish({ entries, hasMore });
    } finally {
      this.reading = false;
    }
  }

  private start(initial: boolean, work: () => Promise<void>): Promise<void> {
    if (this.operation) return this.operation;
    if (this.disposed) return Promise.resolve();
    if (this.snapshot.failed || this.snapshot.moreFailed)
      this.stabilizeChanges();
    this.publish(
      initial
        ? { loading: true, failed: false, moreFailed: false }
        : { loadingMore: true, moreFailed: false },
    );
    const run = async () => {
      try {
        await work();
      } catch {
        this.stabilizeChanges();
        this.publish(initial ? { failed: true } : { moreFailed: true });
      } finally {
        this.publish(initial ? { loading: false } : { loadingMore: false });
      }
    };
    this.operation = run().finally(() => {
      this.operation = null;
      this.ensureBackfill();
    });
    return this.operation;
  }

  private stabilizeChanges(): void {
    // A retry takes a fresh snapshot. Preserve absolute updates/tombstones,
    // but do not replay relative toggles that predate that fresh snapshot.
    const absolute = new Map<number, HistoryUpdatePayload>();
    for (const event of this.changes) {
      if (event.action === "toggled") {
        const previous = absolute.get(event.id);
        if (previous?.action === "added" || previous?.action === "updated") {
          absolute.set(event.id, {
            ...previous,
            entry: { ...previous.entry, saved: !previous.entry.saved },
          });
        }
      } else if (event.action === "updated") {
        if (absolute.get(event.entry.id)?.action !== "deleted")
          absolute.set(event.entry.id, event);
      } else
        absolute.set(
          event.action === "deleted" ? event.id : event.entry.id,
          event,
        );
    }
    const current = new Map(
      this.snapshot.entries.map((entry) => [entry.id, entry]),
    );
    this.changes = [...absolute.values()].map((event) =>
      (event.action === "added" || event.action === "updated") &&
      current.has(event.entry.id)
        ? { ...event, entry: current.get(event.entry.id)! }
        : event,
    );
  }

  prefetch(): Promise<void> {
    return this.start(true, async () => {
      while (
        !this.disposed &&
        this.snapshot.hasMore &&
        this.snapshot.entries.length < USAGE_MAX_RECORDS
      ) {
        await this.readPage(
          this.acceptedPage
            ? Math.min(100, USAGE_MAX_RECORDS - this.snapshot.entries.length)
            : 30,
        );
      }
    });
  }

  loadMore(): Promise<void> {
    if (this.operation) return this.operation;
    if (!this.snapshot.hasMore || this.snapshot.failed || this.disposed)
      return Promise.resolve();
    return this.start(false, () => this.readPage(30));
  }

  receive(event: HistoryUpdatePayload): void {
    if (this.disposed) return;
    if (this.reading || this.snapshot.failed || this.snapshot.moreFailed)
      this.changes.push(event);
    const previousCount = this.snapshot.entries.length;
    this.publish({ entries: applyHistoryEvent(this.snapshot.entries, event) });
    if (
      event.action === "deleted" &&
      this.snapshot.entries.length < previousCount
    )
      this.ensureBackfill();
  }

  private ensureBackfill(): void {
    if (
      this.disposed ||
      this.operation ||
      !this.acceptedPage ||
      this.snapshot.loading ||
      this.snapshot.failed ||
      this.snapshot.moreFailed ||
      !this.snapshot.hasMore ||
      this.snapshot.entries.length >= USAGE_MAX_RECORDS
    )
      return;
    void this.start(false, async () => {
      while (
        !this.disposed &&
        this.snapshot.hasMore &&
        this.snapshot.entries.length < USAGE_MAX_RECORDS
      ) {
        await this.readPage(
          Math.min(100, USAGE_MAX_RECORDS - this.snapshot.entries.length),
        );
      }
    });
  }

  dispose(): void {
    this.disposed = true;
    this.changes = [];
  }
}
