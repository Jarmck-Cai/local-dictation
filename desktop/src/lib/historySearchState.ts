import type { HistoryEntry, HistoryUpdatePayload } from "../bindings";
import type { VocabularyView } from "./dictationApi";
import type {
  HistorySearchPage,
  HistorySearchRequest,
} from "./historySearchApi";

export interface CorrectionDraft {
  text: string;
  lastSubmitted: string | null;
  submitting?: boolean;
  review?: VocabularyView;
}

export interface HistorySearchSnapshot {
  entries: HistoryEntry[];
  input: string;
  query: string;
  composing: boolean;
  invalidQuery: boolean;
  loading: boolean;
  failed: boolean;
  hasMore: boolean;
  page: number;
  anchorId: number | null;
  refreshAvailable: boolean;
  drafts: ReadonlyMap<number, CorrectionDraft>;
  expandedId: number | null;
}

export const recentHistoryEntries = (entries: HistoryEntry[]) =>
  entries.slice(0, 15);

/** Full-history pages never mutate the shared history/usage cache. */
export class HistorySearchState {
  snapshot: HistorySearchSnapshot = {
    entries: [],
    input: "",
    query: "",
    composing: false,
    invalidQuery: false,
    loading: false,
    failed: false,
    hasMore: false,
    page: 1,
    anchorId: null,
    refreshAvailable: false,
    drafts: new Map(),
    expandedId: null,
  };
  private cursors: (number | null)[] = [null];
  private nextCursor: number | null = null;
  private request = 0;
  private started = false;
  private active = false;
  private dirty = false;
  private deleted = new Set<number>();
  private reviewRequests = new Map<number, number>();
  private disposed = false;

  constructor(
    private fetchPage: (
      request: HistorySearchRequest,
    ) => Promise<HistorySearchPage>,
    private onChange: (snapshot: HistorySearchSnapshot) => void = () => {},
  ) {}

  private publish(change: Partial<HistorySearchSnapshot>) {
    if (this.disposed) return;
    this.snapshot = { ...this.snapshot, ...change };
    this.onChange(this.snapshot);
  }

  async setActive(active: boolean): Promise<void> {
    this.active = active;
    if (!active && this.snapshot.composing)
      this.publish({ composing: false, input: this.snapshot.query });
    if (active && (!this.started || this.dirty)) await this.read();
  }

  private async read(): Promise<void> {
    if (this.disposed || this.snapshot.invalidQuery) return;
    const request = ++this.request;
    this.started = true;
    this.dirty = false;
    const cursor = this.cursors[this.cursors.length - 1];
    this.publish({
      entries: [],
      loading: true,
      failed: false,
      page: this.cursors.length,
    });
    try {
      const page = await this.fetchPage({
        cursor,
        limit: 30,
        query: this.snapshot.query || null,
        anchor_id: this.snapshot.anchorId,
      });
      if (this.disposed || request !== this.request) return;
      const entries = page.entries.filter(
        (entry) => !this.deleted.has(entry.id),
      );
      if (!entries.length && this.cursors.length > 1) {
        this.cursors.pop();
        await this.read();
        return;
      }
      this.nextCursor = page.next_cursor;
      this.publish({
        entries,
        loading: false,
        failed: false,
        anchorId: page.anchor_id,
        hasMore:
          page.has_more &&
          page.next_cursor !== null &&
          page.next_cursor !== cursor,
      });
    } catch {
      if (!this.disposed && request === this.request)
        this.publish({ entries: [], loading: false, failed: true });
    }
  }

  async next(): Promise<void> {
    if (
      this.snapshot.loading ||
      this.snapshot.failed ||
      !this.snapshot.hasMore ||
      this.nextCursor === null
    )
      return;
    this.cursors.push(this.nextCursor);
    await this.read();
  }

  async previous(): Promise<void> {
    if (this.snapshot.loading || this.cursors.length === 1) return;
    this.cursors.pop();
    await this.read();
  }

  async refresh(): Promise<void> {
    this.cursors = [null];
    this.nextCursor = null;
    this.publish({ anchorId: null, refreshAvailable: false });
    await this.read();
  }

  retry(): Promise<void> {
    return this.read();
  }

  async setInput(input: string): Promise<void> {
    this.publish({ input });
    if (!this.snapshot.composing) await this.commitInput();
  }

  compositionStart(): void {
    this.publish({ composing: true });
  }

  async compositionEnd(input: string): Promise<void> {
    this.publish({ input, composing: false });
    await this.commitInput();
  }

  private async commitInput(): Promise<void> {
    const invalidQuery = Array.from(this.snapshot.input).length > 256;
    const query = this.snapshot.input.trim() ? this.snapshot.input : "";
    if (
      query === this.snapshot.query &&
      invalidQuery === this.snapshot.invalidQuery
    )
      return;
    ++this.request;
    this.publish({
      query,
      invalidQuery,
      entries: [],
      loading: false,
      failed: false,
      hasMore: false,
    });
    if (!invalidQuery) await this.refresh();
  }

  receive(event: HistoryUpdatePayload): void {
    if (this.disposed) return;
    if (event.action === "added") {
      if (this.started) this.publish({ refreshAvailable: true });
      return;
    }
    if (event.action === "deleted") {
      this.deleted.add(event.id);
      this.reviewRequests.delete(event.id);
      const drafts = new Map(this.snapshot.drafts);
      drafts.delete(event.id);
      this.publish({
        entries: this.snapshot.entries.filter((entry) => entry.id !== event.id),
        drafts,
        expandedId:
          this.snapshot.expandedId === event.id
            ? null
            : this.snapshot.expandedId,
      });
    }
    if (!this.started) return;
    this.dirty = true;
    if (this.active) void this.read();
  }

  setDraftText(id: number, text: string): void {
    if (this.deleted.has(id)) return;
    const drafts = new Map(this.snapshot.drafts);
    drafts.set(id, {
      ...drafts.get(id),
      text,
      lastSubmitted: drafts.get(id)?.lastSubmitted ?? null,
      submitting: drafts.get(id)?.submitting ?? false,
    });
    this.publish({ drafts });
  }

  markSubmitted(id: number, sample: string, review?: VocabularyView): void {
    if (this.deleted.has(id)) return;
    this.reviewRequests.set(id, (this.reviewRequests.get(id) ?? 0) + 1);
    const drafts = new Map(this.snapshot.drafts);
    drafts.set(id, {
      ...drafts.get(id),
      text: drafts.get(id)?.text ?? "",
      lastSubmitted: sample,
      review: review ?? drafts.get(id)?.review,
    });
    this.publish({ drafts });
  }

  setSubmitting(id: number, submitting: boolean): void {
    if (this.deleted.has(id)) return;
    const drafts = new Map(this.snapshot.drafts);
    drafts.set(id, {
      text: "",
      lastSubmitted: null,
      ...drafts.get(id),
      submitting,
    });
    this.publish({ drafts });
  }

  setReview(id: number, review: VocabularyView): void {
    if (this.deleted.has(id)) return;
    this.reviewRequests.set(id, (this.reviewRequests.get(id) ?? 0) + 1);
    const drafts = new Map(this.snapshot.drafts);
    drafts.set(id, {
      text: "",
      lastSubmitted: null,
      ...drafts.get(id),
      review,
    });
    this.publish({ drafts });
  }

  async refreshReview(
    id: number,
    fetchReview: () => Promise<VocabularyView>,
  ): Promise<void> {
    if (
      this.disposed ||
      this.deleted.has(id) ||
      !this.snapshot.drafts.get(id)?.review
    )
      return;
    const request = (this.reviewRequests.get(id) ?? 0) + 1;
    this.reviewRequests.set(id, request);
    try {
      const review = await fetchReview();
      if (!this.disposed && this.reviewRequests.get(id) === request)
        this.setReview(id, review);
    } catch {
      // Keep the last successful result; a remount or vocabulary event retries.
    }
  }

  toggleExpanded(id: number): void {
    if (!this.deleted.has(id))
      this.publish({ expandedId: this.snapshot.expandedId === id ? null : id });
  }

  dispose(): void {
    this.disposed = true;
    ++this.request;
  }
}
