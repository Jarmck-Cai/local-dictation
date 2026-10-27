import { useCallback, useEffect, useRef, useState } from "react";
import { Check, Copy, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { commands, events, type HistoryEntry } from "@/bindings";
import { formatDateTime } from "@/utils/dateFormat";
import { Button } from "../../ui/Button";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { copyToClipboard } from "./clipboard";
import { useDictationMetrics } from "@/hooks/useDictationMetrics";
import type { DictationTiming } from "@/lib/dictationApi";
import { DictationTimingDetails } from "../DictationTiming";
import { CorrectionSample } from "./CorrectionSample";

const PAGE_SIZE = 30;

export const HistorySettings = ({
  latestOnly = false,
}: {
  latestOnly?: boolean;
}) => {
  const { t } = useTranslation();
  const { metrics } = useDictationMetrics();
  const [entries, setEntries] = useState<HistoryEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [failed, setFailed] = useState(false);
  const [hasMore, setHasMore] = useState(true);
  const sentinelRef = useRef<HTMLDivElement>(null);
  const entriesRef = useRef<HistoryEntry[]>([]);
  const loadingRef = useRef(false);
  const requestRef = useRef(0);

  useEffect(() => {
    entriesRef.current = entries;
  }, [entries]);

  const loadPage = useCallback(
    async (cursor?: number) => {
      const first = cursor === undefined;
      if (!first && loadingRef.current) return;
      const request = ++requestRef.current;
      loadingRef.current = true;
      if (first) setLoading(true);
      setFailed(false);
      try {
        const result = await commands.getHistoryEntries(
          cursor ?? null,
          latestOnly ? 1 : PAGE_SIZE,
        );
        if (request !== requestRef.current) return;
        if (result.status !== "ok") throw new Error("history unavailable");
        setEntries((previous) =>
          first
            ? result.data.entries
            : [
                ...previous,
                ...result.data.entries.filter(
                  (entry) => !previous.some((old) => old.id === entry.id),
                ),
              ],
        );
        setHasMore(result.data.has_more);
      } catch {
        if (request === requestRef.current) setFailed(true);
      } finally {
        if (request === requestRef.current) {
          setLoading(false);
          loadingRef.current = false;
        }
      }
    },
    [latestOnly],
  );

  useEffect(() => {
    void loadPage();
    return () => {
      requestRef.current += 1;
    };
  }, [loadPage]);

  useEffect(() => {
    if (loading || latestOnly || !hasMore || failed) return;
    const sentinel = sentinelRef.current;
    if (!sentinel) return;
    const observer = new IntersectionObserver(([entry]) => {
      if (entry.isIntersecting) {
        const last = entriesRef.current[entriesRef.current.length - 1];
        if (last) void loadPage(last.id);
      }
    });
    observer.observe(sentinel);
    return () => observer.disconnect();
  }, [loading, latestOnly, hasMore, failed, loadPage]);

  useEffect(() => {
    let disposed = false;
    let stopListening: (() => void) | undefined;
    const unlisten = events.historyUpdatePayload.listen(({ payload }) => {
      if (payload.action === "added") {
        // A new pipeline result supersedes an in-flight first-page snapshot.
        requestRef.current += 1;
        loadingRef.current = false;
        setLoading(false);
        setFailed(false);
        setEntries((previous) => {
          const next = [
            payload.entry,
            ...previous.filter((entry) => entry.id !== payload.entry.id),
          ];
          return latestOnly ? next.slice(0, 1) : next;
        });
      } else if (payload.action === "updated") {
        setEntries((previous) =>
          previous.map((entry) =>
            entry.id === payload.entry.id ? payload.entry : entry,
          ),
        );
      } else if (payload.action === "deleted") {
        if (latestOnly) void loadPage();
        else
          setEntries((previous) =>
            previous.filter((entry) => entry.id !== payload.id),
          );
      }
    });
    void unlisten.then((stop) => {
      if (disposed) stop();
      else stopListening = stop;
    });
    return () => {
      disposed = true;
      stopListening?.();
    };
  }, [latestOnly, loadPage]);

  const deleteEntry = async (id: number) => {
    const result = await commands.deleteHistoryEntry(id);
    if (result.status !== "ok") throw new Error("delete failed");
    setEntries((previous) => previous.filter((entry) => entry.id !== id));
    if (latestOnly || entriesRef.current.length <= 1) await loadPage();
  };

  return (
    <div className="max-w-3xl w-full mx-auto space-y-4">
      <SettingsGroup
        title={t(
          latestOnly ? "dictation.history.latest" : "dictation.history.title",
        )}
      >
        {loading ? (
          <p className="p-4 text-sm text-text/60">
            {t("settings.history.loading")}
          </p>
        ) : failed ? (
          <div className="p-4 flex items-center justify-between gap-3">
            <p role="alert" className="text-sm text-error">
              {t("dictation.history.loadFailed")}
            </p>
            <Button variant="secondary" onClick={() => loadPage()}>
              {t("dictation.retry")}
            </Button>
          </div>
        ) : entries.length === 0 ? (
          <p className="p-5 text-sm text-text/55">
            {t("dictation.history.empty")}
          </p>
        ) : (
          entries.map((entry) => (
            <HistoryEntryCard
              key={entry.id}
              entry={entry}
              timing={
                latestOnly
                  ? undefined
                  : metrics.find((timing) => timing.history_id === entry.id)
              }
              onDelete={() => deleteEntry(entry.id)}
            />
          ))
        )}
      </SettingsGroup>
      {!latestOnly && (
        <>
          <div ref={sentinelRef} className="h-1" />
          <p className="px-4 text-xs text-text/50">
            {t("dictation.retention")}
          </p>
        </>
      )}
    </div>
  );
};

const HistoryEntryCard = ({
  entry,
  onDelete,
  timing,
}: {
  entry: HistoryEntry;
  onDelete: () => Promise<void>;
  timing?: DictationTiming;
}) => {
  const { t, i18n } = useTranslation();
  const [copied, setCopied] = useState<"final" | "raw" | null>(null);
  const [deleting, setDeleting] = useState(false);
  const timerRef = useRef<ReturnType<typeof setTimeout>>();
  const finalText = entry.post_processed_text?.trim()
    ? entry.post_processed_text
    : entry.transcription_text;

  useEffect(
    () => () => {
      clearTimeout(timerRef.current);
    },
    [],
  );

  const copy = async (kind: "final" | "raw") => {
    const text = kind === "final" ? finalText : entry.transcription_text;
    if (!(await copyToClipboard(text))) {
      toast.error(t("settings.history.copyError"));
      return;
    }
    setCopied(kind);
    clearTimeout(timerRef.current);
    timerRef.current = setTimeout(() => setCopied(null), 2000);
  };

  const remove = async () => {
    setDeleting(true);
    try {
      await onDelete();
    } catch {
      toast.error(t("settings.history.deleteError"));
    } finally {
      setDeleting(false);
    }
  };

  return (
    <article className="p-4 space-y-3">
      <div className="flex items-center justify-between gap-3">
        <time className="text-xs text-text/50">
          {formatDateTime(String(entry.timestamp), i18n.language)}
        </time>
        <div className="flex gap-1.5">
          <Button
            variant="secondary"
            size="sm"
            className="flex items-center gap-1.5"
            onClick={() => copy("final")}
            disabled={deleting || !finalText.trim()}
          >
            {copied === "final" ? <Check size={14} /> : <Copy size={14} />}
            {t(
              copied === "final"
                ? "dictation.history.copied"
                : "dictation.history.copyFinal",
            )}
          </Button>
          <Button
            variant="danger-ghost"
            size="sm"
            onClick={remove}
            disabled={deleting}
            aria-label={t("settings.history.delete")}
            title={t("settings.history.delete")}
          >
            <Trash2 size={15} />
          </Button>
        </div>
      </div>
      <p className="select-text cursor-text whitespace-pre-wrap break-words text-sm leading-relaxed">
        {finalText || t("settings.history.transcriptionFailed")}
      </p>
      {finalText.trim() && (
        <p className="text-xs text-text/50">
          {t("dictation.history.inputAttempt")}
        </p>
      )}
      {entry.post_process_requested && !entry.post_processed_text && (
        <p className="text-xs text-text/55">
          {t("dictation.history.originalUsed")}
        </p>
      )}
      {timing && <DictationTimingDetails timing={timing} />}
      <details className="border-t border-mid-gray/15 pt-2">
        <summary className="text-xs text-text/60 cursor-pointer">
          {t("dictation.history.raw")}
        </summary>
        <div className="pt-3 space-y-3">
          <p className="select-text cursor-text whitespace-pre-wrap break-words text-sm text-text/70">
            {entry.transcription_text ||
              t("settings.history.transcriptionFailed")}
          </p>
          <Button
            variant="secondary"
            size="sm"
            disabled={deleting || !entry.transcription_text.trim()}
            onClick={() => copy("raw")}
          >
            {t(
              copied === "raw"
                ? "dictation.history.copied"
                : "dictation.history.copyRaw",
            )}
          </Button>
        </div>
      </details>
      <CorrectionSample historyId={entry.id} />
    </article>
  );
};
