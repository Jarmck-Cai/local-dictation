import { useEffect, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { dictationApi, type DictationTiming } from "@/lib/dictationApi";
import { mergeTiming } from "@/lib/dictationState";

export function useDictationMetrics() {
  const [metrics, setMetrics] = useState<DictationTiming[]>([]);
  const [failed, setFailed] = useState(false);
  const [reload, setReload] = useState(0);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    setLoading(true);
    let disposed = false;
    let unlisten: UnlistenFn | undefined;
    let snapshotReady = false;
    const updates = new Map<string, DictationTiming>();
    void (async () => {
      try {
        unlisten = await listen<DictationTiming>(
          "dictation-timing",
          ({ payload }) => {
            if (disposed) return;
            if (!snapshotReady) updates.set(payload.id, payload);
            setMetrics((previous) => mergeTiming(previous, payload, 300));
            setFailed(false);
          },
        );
        if (disposed) {
          unlisten();
          return;
        }
        const snapshot = await dictationApi.getMetrics();
        if (disposed) return;
        // Events received while fetching are newer than the snapshot for that id.
        setMetrics(
          [...updates.values()].reduce(
            (items, item) => mergeTiming(items, item, 300),
            snapshot,
          ),
        );
        snapshotReady = true;
        updates.clear();
        setFailed(false);
      } catch {
        if (!disposed) setFailed(true);
      } finally {
        snapshotReady = true;
        updates.clear();
        if (!disposed) setLoading(false);
      }
    })();
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [reload]);

  return {
    metrics,
    failed,
    loading,
    retry: () => setReload((value) => value + 1),
  };
}
