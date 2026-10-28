import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  collectedTermsApi,
  COLLECTED_TERMS_UPDATED,
  type CollectedTermsQuery,
} from "@/lib/collectedTermsApi";
import {
  CollectedTermsState,
  type CollectedTermsSnapshot,
} from "@/lib/collectedTermsState";

export function useCollectedTerms() {
  const controller = useRef<CollectedTermsState | null>(null);
  const [snapshot, setSnapshot] = useState<CollectedTermsSnapshot>({
    query: "",
    filter: "all",
    offset: 0,
    limit: 30,
    page: null,
    loading: true,
    failed: false,
    changeFailed: false,
    busy: false,
  });
  useEffect(() => {
    const state = new CollectedTermsState(collectedTermsApi, setSnapshot);
    controller.current = state;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void (async () => {
      try {
        const stop = await listen(
          COLLECTED_TERMS_UPDATED,
          () => void state.load(),
        );
        if (disposed) {
          stop();
          return;
        }
        unlisten = stop;
      } catch {
        // Manual refresh/retry still works when desktop event registration fails.
      }
      if (!disposed) await state.load();
    })();
    return () => {
      disposed = true;
      state.dispose();
      unlisten?.();
      controller.current = null;
    };
  }, []);
  const load = useCallback((query?: Partial<CollectedTermsQuery>) => {
    void controller.current?.load(query);
  }, []);
  const like = useCallback((id: number, liked: boolean) => {
    void controller.current?.like(id, liked);
  }, []);
  const remove = useCallback((id: number, deleted: boolean) => {
    void controller.current?.remove(id, deleted);
  }, []);
  return { ...snapshot, load, like, remove };
}
