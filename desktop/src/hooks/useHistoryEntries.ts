import { useCallback, useEffect, useRef, useState } from "react";
import { commands, events } from "@/bindings";
import {
  HistoryPager,
  historyUsageStatus,
  type HistoryPagerSnapshot,
} from "@/lib/historyPager";

/** Shared history with bounded initial prefetch and uncapped, on-demand pages. */
export function useHistoryEntries() {
  const [snapshot, setSnapshot] = useState<HistoryPagerSnapshot>({
    entries: [],
    loading: true,
    failed: false,
    hasMore: true,
    loadingMore: false,
    moreFailed: false,
  });
  const [reload, setReload] = useState(0);
  const pagerRef = useRef<HistoryPager | null>(null);
  const subscribedRef = useRef(false);

  useEffect(() => {
    let disposed = false;
    let stopListening: (() => void) | undefined;
    const pager = new HistoryPager(async (cursor, limit) => {
      const result = await commands.getHistoryEntries(cursor, limit);
      if (result.status !== "ok") throw new Error("history unavailable");
      return result.data;
    }, setSnapshot);
    pagerRef.current = pager;
    subscribedRef.current = false;
    void (async () => {
      try {
        // Subscribe before the first snapshot so in-flight pages receive the
        // same absolute added/updated/deleted event overlay as live rows.
        stopListening = await events.historyUpdatePayload.listen(
          ({ payload }) => pager.receive(payload),
        );
        if (disposed) {
          stopListening();
          return;
        }
        subscribedRef.current = true;
        await pager.prefetch();
      } catch {
        if (!disposed)
          setSnapshot((previous) => ({
            ...previous,
            loading: false,
            failed: true,
          }));
      }
    })();
    return () => {
      disposed = true;
      pager.dispose();
      stopListening?.();
      if (pagerRef.current === pager) {
        pagerRef.current = null;
        subscribedRef.current = false;
      }
    };
  }, [reload]);

  const retry = useCallback(() => {
    if (subscribedRef.current && pagerRef.current)
      void pagerRef.current.prefetch();
    else setReload((value) => value + 1);
  }, []);
  const loadMore = useCallback(
    () => pagerRef.current?.loadMore() ?? Promise.resolve(),
    [],
  );
  const removeEntry = useCallback(async (id: number) => {
    const result = await commands.deleteHistoryEntry(id);
    if (result.status !== "ok") throw new Error("delete failed");
    // A duplicate event/acknowledgement is harmless; both use the same backfill
    // and tombstone path, even if the native event arrives after the response.
    pagerRef.current?.receive({ action: "deleted", id });
  }, []);
  return {
    ...snapshot,
    ...historyUsageStatus(snapshot),
    retry,
    loadMore,
    retryMore: loadMore,
    removeEntry,
  };
}
