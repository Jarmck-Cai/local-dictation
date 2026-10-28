import type {
  CollectedTermsTransport,
  CollectedTermsQuery,
  CollectedTermsPage,
} from "./collectedTermsApi";

export interface CollectedTermsSnapshot extends CollectedTermsQuery {
  page: CollectedTermsPage | null;
  loading: boolean;
  failed: boolean;
  changeFailed: boolean;
  busy: boolean;
}

export class CollectedTermsState {
  snapshot: CollectedTermsSnapshot = {
    query: "",
    filter: "all",
    offset: 0,
    limit: 30,
    page: null,
    loading: false,
    failed: false,
    changeFailed: false,
    busy: false,
  };
  private generation = 0;
  private alive = true;
  constructor(
    private api: CollectedTermsTransport,
    private changed: (snapshot: CollectedTermsSnapshot) => void,
  ) {}
  private update(next: Partial<CollectedTermsSnapshot>) {
    if (!this.alive) return;
    this.snapshot = { ...this.snapshot, ...next };
    this.changed(this.snapshot);
  }
  async load(query: Partial<CollectedTermsQuery> = {}): Promise<void> {
    if (!this.alive) return;
    const ticket = ++this.generation;
    this.update(query);
    // Events and searches invalidate old reads but wait for a write to commit.
    if (this.snapshot.busy) return;
    const { query: text, filter, offset, limit } = this.snapshot;
    this.update({ loading: true, failed: false });
    try {
      const page = await this.api.getPage({
        query: text,
        filter,
        offset,
        limit,
      });
      if (!this.alive || ticket !== this.generation) return;
      if (page.entries.length === 0 && offset > 0 && offset >= page.total) {
        await this.load({
          offset: Math.max(0, Math.floor((page.total - 1) / limit) * limit),
        });
        return;
      }
      this.update({ page, loading: false, failed: false });
    } catch {
      if (this.alive && ticket === this.generation)
        this.update({ loading: false, failed: true });
    }
  }
  private async mutate(operation: () => Promise<void>) {
    if (!this.alive || this.snapshot.busy) return;
    ++this.generation;
    this.update({ busy: true, loading: false, changeFailed: false });
    try {
      await operation();
    } catch {
      this.update({ changeFailed: true });
    }
    if (!this.alive) return;
    ++this.generation;
    this.update({ busy: false });
    await this.load();
  }
  like(id: number, liked: boolean) {
    return this.mutate(() => this.api.setLiked(id, liked));
  }
  remove(id: number, deleted: boolean) {
    return this.mutate(() => this.api.setDeleted(id, deleted));
  }
  dispose(): void {
    this.alive = false;
    ++this.generation;
  }
}
