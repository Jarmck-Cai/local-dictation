import { useCallback, useEffect, useRef, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
  dictationApi,
  notifyVocabularyChanged,
  VOCABULARY_REFRESH,
  type VocabularyView,
} from "@/lib/dictationApi";
import { SnapshotGuard } from "@/lib/dictationState";
import { Button } from "../ui/Button";
import { SettingsGroup } from "../ui/SettingsGroup";

export const VocabularyReview = ({
  view,
  onChange,
  showEntries = true,
}: {
  view: VocabularyView;
  onChange: (next: VocabularyView) => void;
  showEntries?: boolean;
}) => {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);
  const guardRef = useRef<SnapshotGuard>();
  useEffect(() => {
    const guard = new SnapshotGuard();
    guardRef.current = guard;
    return () => guard.dispose();
  }, []);

  const mutate = async (operation: () => Promise<VocabularyView>) => {
    const guard = guardRef.current;
    if (!guard || !guard.isAlive() || busy) return;
    const request = guard.begin();
    setBusy(true);
    try {
      const next = await operation();
      if (guard.isCurrent(request)) onChange(next);
      notifyVocabularyChanged();
    } catch {
      if (guard.isCurrent(request))
        toast.error(t("dictation.vocabulary.changeFailed"));
    } finally {
      if (guard.isAlive()) setBusy(false);
    }
  };

  return (
    <div className="space-y-4">
      <div className="space-y-2">
        <h3 className="text-sm font-medium">
          {t("dictation.vocabulary.candidates")}
        </h3>
        <p className="text-xs text-text/60">
          {t("dictation.vocabulary.confirmHint")}
        </p>
        {view.candidates.length === 0 ? (
          <p className="text-sm text-text/55">
            {t("dictation.vocabulary.noCandidates")}
          </p>
        ) : (
          <ul className="space-y-2">
            {view.candidates.map((candidate) => (
              <li
                key={candidate.id}
                className="rounded-lg border border-mid-gray/15 p-3 space-y-2"
              >
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <p className="text-sm select-text break-words">
                    {candidate.replacement_from
                      ? t("dictation.vocabulary.replacement", {
                          from: candidate.replacement_from,
                          term: candidate.term,
                        })
                      : candidate.term}
                  </p>
                  <div className="flex gap-2">
                    <Button
                      size="sm"
                      disabled={busy}
                      onClick={() =>
                        mutate(() =>
                          dictationApi.reviewCandidate(candidate.id, true),
                        )
                      }
                    >
                      {t("dictation.vocabulary.accept")}
                    </Button>
                    <Button
                      variant="secondary"
                      size="sm"
                      disabled={busy}
                      onClick={() =>
                        mutate(() =>
                          dictationApi.reviewCandidate(candidate.id, false),
                        )
                      }
                    >
                      {t("dictation.vocabulary.reject")}
                    </Button>
                  </div>
                </div>
                <p className="text-xs text-text/50">
                  {t("dictation.vocabulary.candidateInfo", {
                    source: t(
                      `dictation.vocabulary.sources.${candidate.source}`,
                      {
                        defaultValue: t(
                          "dictation.vocabulary.sources.transcript",
                        ),
                      },
                    ),
                    count: candidate.occurrences,
                  })}
                </p>
                {candidate.context && (
                  <p className="text-xs text-text/60 whitespace-pre-wrap break-words select-text">
                    {t("dictation.vocabulary.context", {
                      context: candidate.context,
                    })}
                  </p>
                )}
              </li>
            ))}
          </ul>
        )}
      </div>
      {showEntries && (
        <div className="space-y-2 border-t border-mid-gray/15 pt-4">
          <h3 className="text-sm font-medium">
            {t("dictation.vocabulary.learned")}
          </h3>
          <p className="text-xs text-text/60">
            {t("dictation.vocabulary.learnedHint")}
          </p>
          {view.entries.length === 0 ? (
            <p className="text-sm text-text/55">
              {t("dictation.vocabulary.noLearned")}
            </p>
          ) : (
            <ul className="space-y-2">
              {view.entries.map((entry) => (
                <li
                  key={entry.id}
                  className="flex items-start justify-between gap-3 p-2 rounded-lg bg-mid-gray/5"
                >
                  <div className="text-sm select-text break-words">
                    <p>{entry.term}</p>
                    {entry.aliases.length > 0 && (
                      <p className="text-xs text-text/55 mt-1">
                        {t("dictation.vocabulary.aliases", {
                          aliases: entry.aliases.join(
                            t("dictation.vocabulary.separator"),
                          ),
                        })}
                      </p>
                    )}
                    {entry.context && (
                      <p className="text-xs text-text/55 mt-1">
                        {t("dictation.vocabulary.context", {
                          context: entry.context,
                        })}
                      </p>
                    )}
                  </div>
                  <Button
                    size="sm"
                    variant="danger-ghost"
                    disabled={busy}
                    onClick={() =>
                      mutate(() => dictationApi.deleteLearnedTerm(entry.id))
                    }
                  >
                    {t("dictation.vocabulary.delete")}
                  </Button>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </div>
  );
};

export const VocabularyLearning = () => {
  const { t } = useTranslation();
  const [view, setView] = useState<VocabularyView | null>(null);
  const [failed, setFailed] = useState(false);
  const guardRef = useRef<SnapshotGuard>();
  const load = useCallback(async () => {
    const guard = guardRef.current;
    if (!guard || !guard.isAlive()) return;
    const request = guard.begin();
    try {
      const next = await dictationApi.getVocabulary();
      if (guard.isCurrent(request)) {
        setView(next);
        setFailed(false);
      }
    } catch {
      if (guard.isCurrent(request)) setFailed(true);
    }
  }, []);

  useEffect(() => {
    const guard = new SnapshotGuard();
    guardRef.current = guard;
    let unlisten: UnlistenFn | undefined;
    const refresh = () => void load();
    window.addEventListener(VOCABULARY_REFRESH, refresh);
    void (async () => {
      try {
        unlisten = await listen("vocabulary-updated", refresh);
        if (!guard.isAlive()) {
          unlisten();
          return;
        }
      } catch {
        // A manually refreshable snapshot is still useful if event registration fails.
      }
      await load();
    })();
    return () => {
      guard.dispose();
      unlisten?.();
      window.removeEventListener(VOCABULARY_REFRESH, refresh);
    };
  }, [load]);

  return (
    <SettingsGroup
      title={t("dictation.vocabulary.learningTitle")}
      description={t("dictation.vocabulary.learningDescription")}
    >
      <div className="p-4 space-y-4">
        <div className="flex items-center justify-between gap-3">
          <p className="text-xs text-text/60">
            {t("dictation.vocabulary.localOnly")}
          </p>
          <Button variant="secondary" size="sm" onClick={load}>
            {t("dictation.vocabulary.refresh")}
          </Button>
        </div>
        {failed && (
          <p role="alert" className="text-sm text-error">
            {t("dictation.vocabulary.loadFailed")}
          </p>
        )}
        {view ? (
          <VocabularyReview
            view={view}
            onChange={(next) => {
              guardRef.current?.invalidate();
              setView(next);
            }}
          />
        ) : (
          !failed && (
            <p className="text-sm text-text/55">{t("common.loading")}</p>
          )
        )}
      </div>
    </SettingsGroup>
  );
};
