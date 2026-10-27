import { useEffect, useId, useState } from "react";
import { Info } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useDictationData } from "@/hooks/DictationData";
import { summarizeUsage } from "@/lib/usageSummary";
import { Button } from "../ui/Button";
import { LatestDictationTiming } from "./DictationTiming";

export const UsageOverview = () => {
  const { t, i18n } = useTranslation();
  const { history, timing } = useDictationData();
  const [, setClock] = useState(0);
  const now = Date.now();
  const [explaining, setExplaining] = useState(false);
  const explanationId = useId();
  useEffect(() => {
    const timer = setInterval(() => setClock((value) => value + 1), 60_000);
    return () => clearInterval(timer);
  }, []);
  const summary = summarizeUsage(history.entries, timing.metrics, now);
  const loading = history.usageLoading || timing.loading;
  const historyFailed = history.usageFailed;
  const usageReady = !loading && !historyFailed && !timing.failed;
  const number = (value: number) => value.toLocaleString(i18n.language);
  const unavailable = t("dictation.usage.unavailable");
  return (
    <section className="qs-usage" aria-label={t("dictation.usage.title")}>
      <div className="qs-period">
        <span>{t("dictation.usage.period")}</span>
        <span>{t("dictation.usage.scope")}</span>
      </div>
      <dl className="qs-stats" aria-busy={loading}>
        <div className="qs-stat">
          <dt>{t("dictation.usage.transcriptions")}</dt>
          <dd>
            <strong>
              {historyFailed || loading
                ? unavailable
                : number(summary.transcriptions)}
            </strong>
            <span>{t("dictation.usage.times")}</span>
          </dd>
        </div>
        <div className="qs-stat">
          <dt>{t("dictation.usage.generated")}</dt>
          <dd>
            <strong>
              {historyFailed || loading
                ? unavailable
                : number(summary.characters)}
            </strong>
            <span>{t("dictation.usage.characters")}</span>
          </dd>
        </div>
        <div className="qs-stat qs-stat-saved">
          <dt>
            {t("dictation.usage.savedTime")}
            <button
              type="button"
              className="qs-info"
              aria-label={t("dictation.usage.explain")}
              aria-expanded={explaining}
              aria-controls={explanationId}
              onClick={() => setExplaining((value) => !value)}
            >
              <Info size={14} aria-hidden="true" />
            </button>
          </dt>
          <dd>
            <strong>
              {loading ||
              historyFailed ||
              timing.failed ||
              !summary.estimatedSamples
                ? unavailable
                : (summary.savedMs / 60_000).toLocaleString(i18n.language, {
                    maximumFractionDigits: 1,
                  })}
            </strong>
            <span>{t("dictation.usage.minutes")}</span>
          </dd>
          {usageReady && (
            <p className="text-[10px] leading-snug text-text/50">
              {summary.estimatedSamples
                ? t("dictation.usage.estimateSamples", {
                    count: summary.estimatedSamples,
                  })
                : t("dictation.usage.noEstimate")}
            </p>
          )}
        </div>
      </dl>
      <div
        id={explanationId}
        hidden={!explaining}
        className="qs-explanation space-y-1.5"
      >
        <p>{t("dictation.usage.method")}</p>
        <p>
          {usageReady
            ? t("dictation.usage.estimateSamples", {
                count: summary.estimatedSamples,
              })
            : t(loading ? "common.loading" : "dictation.usage.loadFailed")}
        </p>
        <p>{t("dictation.usage.generatedNote")}</p>
        {usageReady && !summary.estimatedSamples && (
          <p>{t("dictation.usage.noEstimate")}</p>
        )}
      </div>
      {(historyFailed || timing.failed) && (
        <div role="alert" className="qs-inline-alert">
          <p>{t("dictation.usage.loadFailed")}</p>
          <Button
            size="sm"
            variant="secondary"
            onClick={() => {
              history.retry();
              timing.retry();
            }}
          >
            {t("dictation.retry")}
          </Button>
        </div>
      )}
    </section>
  );
};

export const UsageSettings = () => (
  <div className="qs-page space-y-6">
    <UsageOverview />
    <LatestDictationTiming />
  </div>
);
