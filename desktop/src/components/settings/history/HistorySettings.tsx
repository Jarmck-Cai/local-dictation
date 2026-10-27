import { useCallback, useEffect, useRef, useState } from "react";
import { Check, ChevronDown, Copy, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import type { HistoryEntry } from "@/bindings";
import { formatDateTime } from "@/utils/dateFormat";
import { useDictationData } from "@/hooks/DictationData";
import { historyFinalText } from "@/lib/usageSummary";
import { Button } from "../../ui/Button";
import { copyToClipboard } from "./clipboard";
import type { DictationTiming } from "@/lib/dictationApi";
import { DictationTimingDetails } from "../DictationTiming";
import { CorrectionSample } from "./CorrectionSample";
import { UsageOverview } from "../UsageOverview";

const PAGE_SIZE = 30;

export const HistorySettings = () => {
  const { t, i18n } = useTranslation();
  const {
    history,
    timing: { metrics },
  } = useDictationData();
  const {
    entries,
    loading,
    failed,
    retry,
    removeEntry,
    hasMore,
    loadingMore,
    moreFailed,
    loadMore,
    retryMore,
  } = history;
  const [visibleCount, setVisibleCount] = useState(PAGE_SIZE);
  const [expandedId, setExpandedId] = useState<number | null>(null);
  const sentinelRef = useRef<HTMLDivElement>(null);

  const revealMore = useCallback(async () => {
    if (loading || loadingMore || failed || moreFailed) return;
    const nextCount = visibleCount + PAGE_SIZE;
    setVisibleCount(nextCount);
    if (nextCount > entries.length && hasMore) await loadMore();
  }, [
    loading,
    loadingMore,
    failed,
    moreFailed,
    visibleCount,
    entries.length,
    hasMore,
    loadMore,
  ]);

  useEffect(() => {
    const sentinel = sentinelRef.current;
    if (
      !sentinel ||
      loading ||
      loadingMore ||
      failed ||
      moreFailed ||
      (visibleCount >= entries.length && !hasMore)
    )
      return;
    const observer = new IntersectionObserver(([entry]) => {
      if (entry.isIntersecting) void revealMore();
    });
    observer.observe(sentinel);
    return () => observer.disconnect();
  }, [
    loading,
    loadingMore,
    failed,
    moreFailed,
    visibleCount,
    entries.length,
    hasMore,
    revealMore,
  ]);

  const today = new Date();
  const yesterday = new Date(today);
  yesterday.setDate(today.getDate() - 1);
  const groups = new Map<string, { label: string; entries: HistoryEntry[] }>();
  for (const entry of entries.slice(0, visibleCount)) {
    const date = new Date(entry.timestamp * 1000);
    const key = date.toDateString();
    const label =
      key === today.toDateString()
        ? t("dictation.history.today")
        : key === yesterday.toDateString()
          ? t("dictation.history.yesterday")
          : date.toLocaleDateString(i18n.language, {
              month: "long",
              day: "numeric",
            });
    if (!groups.has(key)) groups.set(key, { label, entries: [] });
    groups.get(key)!.entries.push(entry);
  }

  return (
    <div className="qs-page space-y-6">
      <UsageOverview />
      <section aria-label={t("dictation.history.title")}>
        {loading && !entries.length && (
          <p role="status" className="qs-empty">
            {t("settings.history.loading")}
          </p>
        )}
        {failed && (
          <div className="qs-inline-alert" role="alert">
            <p>{t("dictation.history.loadFailed")}</p>
            <Button variant="secondary" onClick={retry}>
              {t("dictation.retry")}
            </Button>
          </div>
        )}
        {!loading && !failed && !entries.length && (
          <p className="qs-empty">{t("dictation.history.empty")}</p>
        )}
        {[...groups].map(([key, group]) => (
          <section key={key} className="qs-day-group" aria-label={group.label}>
            <div className="qs-day-heading">
              <h2>{group.label}</h2>
              <span>
                {t("dictation.history.recordCount", {
                  count: group.entries.length,
                })}
              </span>
            </div>
            {group.entries.map((entry) => (
              <HistoryEntryCard
                key={entry.id}
                entry={entry}
                timing={metrics.find((row) => row.history_id === entry.id)}
                expanded={expandedId === entry.id}
                onToggle={() =>
                  setExpandedId((id) => (id === entry.id ? null : entry.id))
                }
                onDelete={() => removeEntry(entry.id)}
              />
            ))}
          </section>
        ))}
        <div ref={sentinelRef} className="h-1" />
        {moreFailed && (
          <div className="qs-inline-alert" role="alert">
            <p>{t("dictation.history.loadMoreFailed")}</p>
            <Button variant="secondary" size="sm" onClick={() => retryMore()}>
              {t("dictation.retry")}
            </Button>
          </div>
        )}
        {!moreFailed && (visibleCount < entries.length || hasMore) && (
          <div className="py-3 text-center">
            <Button
              variant="ghost"
              size="sm"
              onClick={revealMore}
              disabled={loading || loadingMore || failed}
            >
              {t(loadingMore ? "common.loading" : "dictation.history.loadMore")}
            </Button>
          </div>
        )}
      </section>
      <p className="text-xs text-text/45 leading-relaxed">
        {t("dictation.retention")}
      </p>
    </div>
  );
};

const HistoryEntryCard = ({
  entry,
  onDelete,
  timing,
  expanded,
  onToggle,
}: {
  entry: HistoryEntry;
  onDelete: () => Promise<void>;
  timing?: DictationTiming;
  expanded: boolean;
  onToggle: () => void;
}) => {
  const { t, i18n } = useTranslation();
  const [copied, setCopied] = useState<"final" | "raw" | null>(null);
  const [deleting, setDeleting] = useState(false);
  const timerRef = useRef<ReturnType<typeof setTimeout>>();
  const finalText = historyFinalText(entry);
  const detailsId = `history-details-${entry.id}`;
  useEffect(() => () => clearTimeout(timerRef.current), []);

  const copy = async (kind: "final" | "raw") => {
    if (
      !(await copyToClipboard(
        kind === "final" ? finalText : entry.transcription_text,
      ))
    ) {
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
    <article className={`qs-record ${expanded ? "qs-record-expanded" : ""}`}>
      <div className="qs-record-row">
        <button
          type="button"
          className="qs-record-toggle"
          aria-expanded={expanded}
          aria-controls={detailsId}
          onClick={onToggle}
        >
          <time
            title={formatDateTime(String(entry.timestamp), i18n.language)}
            dateTime={new Date(entry.timestamp * 1000).toISOString()}
          >
            {new Date(entry.timestamp * 1000).toLocaleTimeString(
              i18n.language,
              { hour: "2-digit", minute: "2-digit" },
            )}
          </time>
          <span
            className={`qs-record-preview ${expanded ? "qs-record-collapse" : ""}`}
          >
            {expanded
              ? t("dictation.history.collapse")
              : finalText || t("settings.history.transcriptionFailed")}
          </span>
          <ChevronDown
            size={15}
            className="qs-record-chevron"
            aria-hidden="true"
          />
        </button>
        <Button
          variant="ghost"
          size="sm"
          className="qs-copy"
          onClick={() => copy("final")}
          disabled={deleting || !finalText.trim()}
        >
          {copied === "final" ? (
            <Check size={13} aria-hidden="true" />
          ) : (
            <Copy size={13} aria-hidden="true" />
          )}
          {t(
            copied === "final"
              ? "dictation.history.copied"
              : "dictation.history.copy",
          )}
        </Button>
      </div>
      <div
        id={detailsId}
        hidden={!expanded}
        className="qs-record-details space-y-4"
      >
        <p className="select-text cursor-text whitespace-pre-wrap break-words text-sm leading-relaxed">
          {finalText || t("settings.history.transcriptionFailed")}
        </p>
        {entry.post_process_requested && !entry.post_processed_text && (
          <p className="text-xs text-text/55">
            {t("dictation.history.originalUsed")}
          </p>
        )}
        <details className="qs-detail-section">
          <summary>{t("dictation.history.raw")}</summary>
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
        {timing && (
          <details className="qs-detail-section">
            <summary>{t("dictation.history.timing")}</summary>
            <div className="pt-3">
              <DictationTimingDetails timing={timing} />
            </div>
          </details>
        )}
        {finalText.trim() && <CorrectionSample historyId={entry.id} />}
        <div className="flex items-center justify-between gap-4 border-t border-mid-gray/15 pt-3">
          <p className="text-xs text-text/45">
            {t("dictation.history.inputAttempt")}
          </p>
          <Button
            variant="danger-ghost"
            size="sm"
            onClick={remove}
            disabled={deleting}
            className="flex items-center gap-1.5 shrink-0"
          >
            <Trash2 size={13} aria-hidden="true" />
            {t("settings.history.delete")}
          </Button>
        </div>
      </div>
    </article>
  );
};
