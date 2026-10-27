import { useEffect, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { dictationApi, type DictationTiming } from "@/lib/dictationApi";
import { mergeTiming } from "@/lib/dictationState";

export function useDictationMetrics() {
  const [metrics, setMetrics] = useState<DictationTiming[]>([]);
  const [failed, setFailed] = useState(false);
  const [reload, setReload] = useState(0);

  useEffect(() => {
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
            setMetrics((previous) => mergeTiming(previous, payload));
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
            (items, item) => mergeTiming(items, item),
            snapshot,
          ),
        );
        snapshotReady = true;
        updates.clear();
        setFailed(false);
      } catch {
        if (!disposed) setFailed(true);
      }
    })();
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [reload]);

  return { metrics, failed, retry: () => setReload((value) => value + 1) };
}
