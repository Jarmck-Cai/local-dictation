import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Heart } from "lucide-react";
import { useCollectedTerms } from "@/hooks/useCollectedTerms";
import type { CollectedTermsFilter } from "@/lib/collectedTermsApi";
import { Button } from "../ui/Button";
import { Input } from "../ui/Input";
import { SettingsGroup } from "../ui/SettingsGroup";

export const CollectedTerms = () => {
  const { t } = useTranslation();
  const terms = useCollectedTerms();
  const [draft, setDraft] = useState("");
  const [composing, setComposing] = useState(false);
  const { load, query } = terms;
  useEffect(() => {
    if (composing || draft === query) return;
    const timer = window.setTimeout(
      () => load({ query: draft, offset: 0 }),
      200,
    );
    return () => window.clearTimeout(timer);
  }, [draft, composing, query, load]);
  const page = terms.page;
  const pending = terms.loading || terms.busy;

  return (
    <SettingsGroup
      title={t("dictation.collectedTerms.title")}
      description={t("dictation.collectedTerms.description")}
    >
      <div className="p-4 space-y-4">
        <p className="text-xs text-text/60">
          {t("dictation.collectedTerms.boundary")}
        </p>
        <div className="flex flex-wrap gap-2 items-center">
          <Input
            className="flex-1 min-w-36"
            type="search"
            value={draft}
            aria-label={t("dictation.collectedTerms.search")}
            placeholder={t("dictation.collectedTerms.search")}
            onChange={(event) =>
              setDraft([...event.target.value].slice(0, 256).join(""))
            }
            onCompositionStart={() => setComposing(true)}
            onCompositionEnd={(event) => {
              setComposing(false);
              setDraft([...event.currentTarget.value].slice(0, 256).join(""));
            }}
          />
          <Button
            variant="secondary"
            size="sm"
            disabled={terms.busy}
            onClick={() => load()}
          >
            {t("dictation.collectedTerms.refresh")}
          </Button>
        </div>
        <div
          className="flex flex-wrap gap-2"
          role="group"
          aria-label={t("dictation.collectedTerms.filterLabel")}
        >
          {(["all", "liked", "deleted"] as CollectedTermsFilter[]).map(
            (filter) => (
              <Button
                key={filter}
                variant={terms.filter === filter ? "primary-soft" : "ghost"}
                size="sm"
                aria-pressed={terms.filter === filter}
                onClick={() => load({ filter, offset: 0 })}
              >
                {t(`dictation.collectedTerms.filters.${filter}`)}
              </Button>
            ),
          )}
        </div>
        {terms.failed && (
          <div
            role="alert"
            className="flex flex-wrap items-center gap-2 text-sm text-error"
          >
            <span>{t("dictation.collectedTerms.loadFailed")}</span>
            <Button variant="secondary" size="sm" onClick={() => load()}>
              {t("dictation.collectedTerms.retry")}
            </Button>
          </div>
        )}
        {terms.changeFailed && (
          <p role="alert" className="text-sm text-error">
            {t("dictation.collectedTerms.changeFailed")}
          </p>
        )}
        <div aria-busy={pending} aria-live="polite">
          {terms.loading ? (
            <p className="text-sm text-text/55">{t("common.loading")}</p>
          ) : (
            page &&
            (page.entries.length === 0 ? (
              <p className="text-sm text-text/55">
                {t(`dictation.collectedTerms.empty.${terms.filter}`)}
              </p>
            ) : (
              <ul className="divide-y divide-mid-gray/15">
                {page.entries.map((entry) => (
                  <li
                    key={entry.id}
                    className="py-3 flex flex-wrap items-center justify-between gap-3"
                  >
                    <div className="min-w-0 flex-1">
                      <p className="text-sm font-medium break-words select-text">
                        {entry.term}
                      </p>
                      <p className="text-xs text-text/55 mt-1">
                        {t("dictation.collectedTerms.recordCount", {
                          count: entry.record_count,
                        })}
                      </p>
                    </div>
                    <div className="flex gap-2 items-center">
                      <Button
                        variant={entry.liked ? "primary-soft" : "ghost"}
                        size="sm"
                        disabled={pending}
                        aria-pressed={entry.liked}
                        aria-label={t(
                          entry.liked
                            ? "dictation.collectedTerms.unlikeTerm"
                            : "dictation.collectedTerms.likeTerm",
                          { term: entry.term },
                        )}
                        onClick={() => terms.like(entry.id, !entry.liked)}
                      >
                        <span className="inline-flex items-center gap-1">
                          <Heart
                            size={14}
                            aria-hidden="true"
                            fill={entry.liked ? "currentColor" : "none"}
                          />
                          {t(
                            entry.liked
                              ? "dictation.collectedTerms.unlike"
                              : "dictation.collectedTerms.like",
                          )}
                        </span>
                      </Button>
                      <Button
                        variant={entry.deleted ? "secondary" : "danger-ghost"}
                        size="sm"
                        disabled={pending}
                        aria-label={t(
                          entry.deleted
                            ? "dictation.collectedTerms.restoreTerm"
                            : "dictation.collectedTerms.deleteTerm",
                          { term: entry.term },
                        )}
                        onClick={() => terms.remove(entry.id, !entry.deleted)}
                      >
                        {t(
                          entry.deleted
                            ? "dictation.collectedTerms.restore"
                            : "dictation.collectedTerms.delete",
                        )}
                      </Button>
                    </div>
                  </li>
                ))}
              </ul>
            ))
          )}
        </div>
        {page && (
          <div className="flex flex-wrap items-center justify-between gap-2">
            <span className="text-xs text-text/55">
              {t("dictation.collectedTerms.page", {
                page: Math.floor(terms.offset / terms.limit) + 1,
                pages: Math.max(1, Math.ceil(page.total / terms.limit)),
                count: page.total,
              })}
            </span>
            <div className="flex gap-2">
              <Button
                variant="secondary"
                size="sm"
                disabled={pending || terms.offset === 0}
                onClick={() =>
                  load({ offset: Math.max(0, terms.offset - terms.limit) })
                }
              >
                {t("dictation.collectedTerms.previous")}
              </Button>
              <Button
                variant="secondary"
                size="sm"
                disabled={pending || terms.offset + terms.limit >= page.total}
                onClick={() => load({ offset: terms.offset + terms.limit })}
              >
                {t("dictation.collectedTerms.next")}
              </Button>
            </div>
          </div>
        )}
        <p className="text-xs text-text/50">
          {t("dictation.collectedTerms.coverage")}
        </p>
      </div>
    </SettingsGroup>
  );
};
