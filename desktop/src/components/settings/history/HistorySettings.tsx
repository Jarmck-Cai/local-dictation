import { useEffect, useRef, useState } from "react";
import { Check, ChevronDown, Copy, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import type { HistoryEntry } from "@/bindings";
import { formatDateTime } from "@/utils/dateFormat";
import { useDictationData } from "@/hooks/DictationData";
import { useHistorySearch } from "@/hooks/useHistorySearch";
import {
  recentHistoryEntries,
  type CorrectionDraft,
} from "@/lib/historySearchState";
import { historyFinalText } from "@/lib/usageSummary";
import { Button } from "../../ui/Button";
import { copyToClipboard } from "./clipboard";
import type { DictationTiming } from "@/lib/dictationApi";
import { DictationTimingDetails } from "../DictationTiming";
import { CorrectionSample } from "./CorrectionSample";
import { UsageOverview } from "../UsageOverview";
import "./historyWorkspace.css";

export const HistorySettings = () => {
  const { t, i18n } = useTranslation();
  const {
    history,
    timing: { metrics },
  } = useDictationData();
  const [allHistory, setAllHistory] = useState(false);
  const search = useHistorySearch(allHistory);
  const { state, expandedId } = search;
  const entries = allHistory
    ? search.entries
    : recentHistoryEntries(history.entries);
  const loading = allHistory ? search.loading : history.loading;
  const failed = allHistory ? search.failed : history.failed;
  const retry = allHistory ? search.retry : history.retry;

  const today = new Date();
  const yesterday = new Date(today);
  yesterday.setDate(today.getDate() - 1);
  const groups = new Map<string, { label: string; entries: HistoryEntry[] }>();
  for (const entry of entries) {
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
        <div className="qs-history-toolbar">
          <div>
            <h1>
              {t(
                allHistory
                  ? "dictation.historyWorkspace.title"
                  : "dictation.historyWorkspace.recent",
              )}
            </h1>
            <p className="qs-history-caption">
              {t(
                allHistory
                  ? "dictation.historyWorkspace.pageHint"
                  : "dictation.historyWorkspace.recentHint",
              )}
            </p>
          </div>
          <div className="flex flex-wrap gap-2">
            {allHistory && (
              <Button
                variant="secondary"
                size="sm"
                onClick={() => void state.refresh()}
                disabled={loading || search.invalidQuery || search.composing}
              >
                {t("dictation.historyWorkspace.refresh")}
              </Button>
            )}
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setAllHistory((value) => !value)}
            >
              {t(
                allHistory
                  ? "dictation.historyWorkspace.back"
                  : "dictation.historyWorkspace.allHistory",
              )}
            </Button>
          </div>
        </div>
        {allHistory && (
          <div className="qs-history-search">
            <label htmlFor="history-search">
              {t("dictation.historyWorkspace.searchLabel")}
            </label>
            <input
              id="history-search"
              type="search"
              value={search.input}
              aria-describedby="history-search-help"
              aria-invalid={search.invalidQuery}
              placeholder={t("dictation.historyWorkspace.searchPlaceholder")}
              onChange={(event) => void state.setInput(event.target.value)}
              onCompositionStart={() => state.compositionStart()}
              onCompositionEnd={(event) =>
                void state.compositionEnd(event.currentTarget.value)
              }
            />
            <p
              id="history-search-help"
              className={
                search.invalidQuery
                  ? "text-xs text-error"
                  : "qs-history-caption"
              }
              role={search.invalidQuery ? "alert" : undefined}
            >
              {t(
                search.invalidQuery
                  ? "dictation.historyWorkspace.queryTooLong"
                  : "dictation.historyWorkspace.searchHelp",
              )}
            </p>
            {search.refreshAvailable && (
              <p role="status" className="text-xs text-text/65">
                {t("dictation.historyWorkspace.newRecords")}
              </p>
            )}
          </div>
        )}
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
        {!loading &&
          !failed &&
          !entries.length &&
          !(allHistory && search.invalidQuery) && (
            <p className="qs-empty">
              {t(
                allHistory && search.query
                  ? "dictation.historyWorkspace.noMatches"
                  : "dictation.history.empty",
              )}
            </p>
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
                onToggle={() => state.toggleExpanded(entry.id)}
                draft={search.drafts.get(entry.id)}
                onDraftChange={(text) => state.setDraftText(entry.id, text)}
                onSubmitted={(sample) => state.markSubmitted(entry.id, sample)}
                onSubmittingChange={(submitting) =>
                  state.setSubmitting(entry.id, submitting)
                }
                onDelete={async () => {
                  await history.removeEntry(entry.id);
                  state.receive({ action: "deleted", id: entry.id });
                }}
              />
            ))}
          </section>
        ))}
        {allHistory && (
          <nav
            className="qs-history-pagination"
            aria-label={t("dictation.historyWorkspace.title")}
          >
            <Button
              variant="secondary"
              size="sm"
              onClick={() => void state.previous()}
              disabled={
                loading ||
                search.page <= 1 ||
                search.invalidQuery ||
                search.composing
              }
            >
              {t("dictation.historyWorkspace.previous")}
            </Button>
            <span aria-live="polite">
              {t("dictation.historyWorkspace.page", { page: search.page })}
            </span>
            <Button
              variant="secondary"
              size="sm"
              onClick={() => void state.next()}
              disabled={
                loading ||
                failed ||
                !search.hasMore ||
                search.invalidQuery ||
                search.composing
              }
            >
              {t("dictation.historyWorkspace.next")}
            </Button>
          </nav>
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
  draft,
  onDraftChange,
  onSubmitted,
  onSubmittingChange,
}: {
  entry: HistoryEntry;
  onDelete: () => Promise<void>;
  timing?: DictationTiming;
  expanded: boolean;
  onToggle: () => void;
  draft: CorrectionDraft | undefined;
  onDraftChange: (text: string) => void;
  onSubmitted: (sample: string) => void;
  onSubmittingChange: (submitting: boolean) => void;
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
        {finalText.trim() && (
          <CorrectionSample
            historyId={entry.id}
            draft={draft}
            onTextChange={onDraftChange}
            onSubmitted={onSubmitted}
            onSubmittingChange={onSubmittingChange}
          />
        )}
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
