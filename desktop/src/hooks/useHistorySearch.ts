import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { events } from "@/bindings";
import { searchHistoryEntries } from "@/lib/historySearchApi";
import { HistorySearchState } from "@/lib/historySearchState";

/** Subscribe even in recent mode so removed records also lose their drafts. */
export function useHistorySearch(active: boolean) {
  const [store] = useState(() => {
    const listeners = new Set<() => void>();
    const state = new HistorySearchState(searchHistoryEntries, () => {
      for (const notify of listeners) notify();
    });
    return {
      state,
      subscribe: (notify: () => void) => {
        listeners.add(notify);
        return () => {
          listeners.delete(notify);
        };
      },
    };
  });
  const snapshot = useSyncExternalStore(
    store.subscribe,
    () => store.state.snapshot,
  );
  const [subscriptionFailed, setSubscriptionFailed] = useState(false);
  const [connecting, setConnecting] = useState(true);
  const [reload, setReload] = useState(0);
  const ready = useRef(false);
  const activeRef = useRef(active);

  useEffect(() => {
    let disposed = false;
    let stop: (() => void) | undefined;
    ready.current = false;
    setConnecting(true);
    setSubscriptionFailed(false);
    void events.historyUpdatePayload
      .listen(({ payload }) => store.state.receive(payload))
      .then((unlisten) => {
        if (disposed) {
          unlisten();
          return;
        }
        stop = unlisten;
        ready.current = true;
        setConnecting(false);
        void store.state.setActive(activeRef.current);
      })
      .catch(() => {
        if (disposed) return;
        setConnecting(false);
        setSubscriptionFailed(true);
      });
    return () => {
      disposed = true;
      ready.current = false;
      stop?.();
      void store.state.setActive(false);
    };
  }, [store, reload]);

  useEffect(() => {
    activeRef.current = active;
    if (ready.current) void store.state.setActive(active);
  }, [active, store]);

  return {
    ...snapshot,
    loading: snapshot.loading || connecting,
    failed: snapshot.failed || subscriptionFailed,
    state: store.state,
    retry: () =>
      subscriptionFailed
        ? setReload((value) => value + 1)
        : void store.state.retry(),
  };
}
