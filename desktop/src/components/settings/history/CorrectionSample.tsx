import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  dictationApi,
  notifyVocabularyChanged,
  VOCABULARY_REFRESH,
  type VocabularyView,
} from "@/lib/dictationApi";
import { SnapshotGuard } from "@/lib/dictationState";
import type { CorrectionDraft } from "@/lib/historySearchState";
import { Button } from "../../ui/Button";
import { VocabularyReview } from "../VocabularyLearning";

const MAX_SAMPLE_CHARACTERS = 8000;

interface CorrectionSampleProps {
  historyId: number;
  draft: CorrectionDraft | undefined;
  onTextChange: (text: string) => void;
  onSubmitted: (sample: string, review: VocabularyView) => void;
  onSubmittingChange: (submitting: boolean) => void;
  onReviewChange: (review: VocabularyView) => void;
  onReviewRefresh: () => void;
}

export const CorrectionSample = (props: CorrectionSampleProps) => {
  return <CorrectionSampleForm key={props.historyId} {...props} />;
};

const CorrectionSampleForm = ({
  historyId,
  draft,
  onTextChange,
  onSubmitted,
  onSubmittingChange,
  onReviewChange,
  onReviewRefresh,
}: CorrectionSampleProps) => {
  const { t } = useTranslation();
  const text = draft?.text ?? "";
  const lastSubmitted = draft?.lastSubmitted ?? null;
  const busy = draft?.submitting ?? false;
  const [failed, setFailed] = useState(false);
  const view = draft?.review ?? null;
  const refreshReviewRef = useRef(onReviewRefresh);
  refreshReviewRef.current = onReviewRefresh;
  const hasReview = view !== null;
  const guardRef = useRef<SnapshotGuard>();
  const characterCount = Array.from(text.trim()).length;

  useEffect(() => {
    const guard = new SnapshotGuard();
    guardRef.current = guard;
    return () => guard.dispose();
  }, [historyId]);

  useEffect(() => {
    if (!hasReview) return;
    // A candidate action may finish while this record is on another page.
    const refresh = () => refreshReviewRef.current();
    refresh();
    window.addEventListener(VOCABULARY_REFRESH, refresh);
    return () => window.removeEventListener(VOCABULARY_REFRESH, refresh);
  }, [historyId, hasReview]);

  const submit = async () => {
    const guard = guardRef.current;
    if (
      !guard ||
      busy ||
      characterCount === 0 ||
      characterCount > MAX_SAMPLE_CHARACTERS
    )
      return;
    const request = guard.begin();
    const sample = text.trim();
    onSubmittingChange(true);
    setFailed(false);
    try {
      const next = await dictationApi.submitCorrection(historyId, sample);
      onSubmitted(sample, next);
      notifyVocabularyChanged();
    } catch {
      if (guard.isCurrent(request)) setFailed(true);
    } finally {
      onSubmittingChange(false);
    }
  };

  return (
    <section
      className="qs-correction space-y-2.5"
      aria-label={t("dictation.correction.title")}
    >
      <p className="text-xs text-text/65">
        {t("dictation.correction.description")}
      </p>
      <label className="block text-sm">
        <span className="sr-only">{t("dictation.correction.label")}</span>
        <textarea
          value={text}
          rows={2}
          maxLength={MAX_SAMPLE_CHARACTERS * 2}
          disabled={busy}
          className="block w-full resize-y rounded-lg border border-mid-gray/25 bg-background p-2.5 text-sm text-text select-text focus:outline-none focus:border-logo-primary disabled:opacity-60"
          placeholder={t("dictation.correction.placeholder")}
          onChange={(event) => {
            onTextChange(event.target.value);
            setFailed(false);
          }}
        />
      </label>
      <div className="flex flex-wrap items-center justify-between gap-3">
        <p
          className={
            characterCount > MAX_SAMPLE_CHARACTERS
              ? "text-xs text-error"
              : "text-xs text-text/50"
          }
        >
          {t("dictation.correction.characters", {
            count: characterCount,
            limit: MAX_SAMPLE_CHARACTERS,
          })}
        </p>
        <Button
          size="sm"
          disabled={
            busy ||
            characterCount === 0 ||
            characterCount > MAX_SAMPLE_CHARACTERS ||
            text.trim() === lastSubmitted
          }
          onClick={submit}
        >
          {t(
            busy
              ? "dictation.correction.submitting"
              : "dictation.correction.submit",
          )}
        </Button>
      </div>
      <details className="qs-detail-section">
        <summary>{t("dictation.correction.dataHelp")}</summary>
        <p className="pt-2 text-xs text-text/50">
          {t("dictation.correction.privacy")}
        </p>
      </details>
      {failed && (
        <p role="alert" className="text-sm text-error">
          {t("dictation.correction.failed")}
        </p>
      )}
      {view && (
        <div className="space-y-3 border-t border-mid-gray/15 pt-3">
          <p role="status" className="text-sm text-text/75">
            {t("dictation.correction.saved")}
          </p>
          <VocabularyReview
            view={view}
            onChange={onReviewChange}
            showEntries={false}
          />
        </div>
      )}
    </section>
  );
};
