import { useTranslation } from "react-i18next";
import { useDictationData } from "@/hooks/DictationData";
import type { DictationTiming } from "@/lib/dictationApi";
import { distinctCleanupReason } from "@/lib/dictationState";
import { summarizeResources, summarizeTimings } from "@/lib/dictationSummary";
import { Button } from "../ui/Button";
import { SettingsGroup } from "../ui/SettingsGroup";

export const DictationTimingDetails = ({
  timing,
}: {
  timing: DictationTiming;
}) => {
  const { t, i18n } = useTranslation();
  const cleanupReason = distinctCleanupReason(timing);
  const resources = summarizeResources(timing);
  const seconds = (milliseconds: number) =>
    t("dictation.timing.seconds", {
      value: (milliseconds / 1000).toLocaleString(i18n.language, {
        maximumFractionDigits: 2,
      }),
    });
  const values = [
    [
      "recording",
      typeof timing.recording_ms === "number" &&
      Number.isFinite(timing.recording_ms) &&
      timing.recording_ms >= 0
        ? seconds(timing.recording_ms)
        : t("dictation.timing.unavailable"),
    ],
    ["asr", seconds(timing.asr_ms)],
    ["cleanup", seconds(timing.cleanup_ms)],
    [
      "paste",
      timing.paste_ms === null
        ? t("dictation.timing.insertionUnconfirmed")
        : seconds(timing.paste_ms),
    ],
    ["total", seconds(timing.total_ms)],
  ];
  return (
    <div className="space-y-2 text-xs text-text/65">
      <dl className="grid grid-cols-2 sm:grid-cols-4 gap-2">
        {values.map(([name, value]) => (
          <div key={name}>
            <dt>{t(`dictation.timing.${name}`)}</dt>
            <dd className="mt-0.5 font-medium text-text">{value}</dd>
          </div>
        ))}
      </dl>
      <p>
        {t("dictation.timing.result", {
          mode: t(`dictation.cleanup.modes.${timing.mode}`, {
            defaultValue: timing.mode,
          }),
          outcome: t(`dictation.timing.outcomes.${timing.outcome}`, {
            defaultValue: t("dictation.timing.outcomes.unknown"),
          }),
        })}
      </p>
      {cleanupReason && (
        <p>
          {t("dictation.timing.cleanupFallback", {
            reason: t(`dictation.timing.outcomes.${cleanupReason}`, {
              defaultValue: t("dictation.timing.cleanupFallbackUnknown"),
            }),
          })}
        </p>
      )}
      {timing.cleanup_budget_ms > 0 && (
        <p>
          {t("dictation.timing.budget", {
            value: seconds(timing.cleanup_budget_ms),
          })}
        </p>
      )}
      {resources.samples > 0 && (
        <div className="space-y-1 border-t border-mid-gray/20 pt-2">
          <dl className="grid grid-cols-2 gap-2">
            <div>
              <dt>{t("dictation.timing.sampledMemory")}</dt>
              <dd className="mt-0.5 font-medium text-text">
                {resources.sampledWorkingSetBytes === null
                  ? t("dictation.timing.unavailable")
                  : t("dictation.timing.mebibytes", {
                      value: (
                        resources.sampledWorkingSetBytes /
                        1024 /
                        1024
                      ).toLocaleString(i18n.language, {
                        maximumFractionDigits: 1,
                      }),
                    })}
              </dd>
            </div>
            <div>
              <dt>{t("dictation.timing.processCpu")}</dt>
              <dd className="mt-0.5 font-medium text-text">
                {resources.processCpuMs === null
                  ? t("dictation.timing.unavailable")
                  : seconds(resources.processCpuMs)}
              </dd>
            </div>
          </dl>
          <p className="text-text/45">{t("dictation.timing.resourceNote")}</p>
        </div>
      )}
      <p className="text-text/45">{t("dictation.timing.note")}</p>
    </div>
  );
};

const DictationTimingSummary = ({
  metrics,
}: {
  metrics: DictationTiming[];
}) => {
  const { t, i18n } = useTranslation();
  const summary = summarizeTimings(metrics, Date.now());
  const seconds = (value: number) =>
    t("dictation.timing.seconds", {
      value: (value / 1000).toLocaleString(i18n.language, {
        maximumFractionDigits: 2,
      }),
    });
  const counts = [
    ["completed", summary.completed.length],
    [
      "fallbacks",
      summary.fallbackReasons.reduce((sum, item) => sum + item.count, 0),
    ],
    ["cancelled", summary.cancelled],
    ["noAudio", summary.noAudio],
    ["unconfirmed", summary.unconfirmed],
  ] as const;
  return (
    <div className="space-y-4">
      <p className="text-xs text-text/55">
        {t("dictation.timing.summaryScope", { count: summary.records.length })}
      </p>
      <dl className="grid grid-cols-2 sm:grid-cols-5 gap-3 text-xs text-text/65">
        {counts.map(([name, count]) => (
          <div key={name}>
            <dt>{t(`dictation.timing.counts.${name}`)}</dt>
            <dd className="mt-1 font-medium text-text">{count}</dd>
          </div>
        ))}
      </dl>
      {summary.medians ? (
        <div className="space-y-2">
          <h3 className="text-sm font-medium">
            {t("dictation.timing.medianTitle", {
              count: summary.completed.length,
            })}
          </h3>
          <dl className="grid grid-cols-2 sm:grid-cols-4 gap-2 text-xs text-text/65">
            {Object.entries(summary.medians).map(([name, value]) => (
              <div key={name}>
                <dt>{t(`dictation.timing.${name}`)}</dt>
                <dd className="mt-0.5 font-medium text-text">
                  {seconds(value)}
                </dd>
              </div>
            ))}
          </dl>
          {summary.smallSample && (
            <p className="text-xs text-text/55">
              {t("dictation.timing.smallSample")}
            </p>
          )}
        </div>
      ) : (
        <p className="text-xs text-text/55">
          {t("dictation.timing.noCompletedSamples")}
        </p>
      )}
      {summary.fallbackReasons.length > 0 && (
        <div className="space-y-1 text-xs text-text/65">
          <h3 className="text-sm font-medium text-text">
            {t("dictation.timing.fallbackTitle")}
          </h3>
          <ul className="space-y-1">
            {summary.fallbackReasons.map(({ reason, count }) => (
              <li key={reason}>
                {t("dictation.timing.reasonCount", {
                  reason: t(`dictation.timing.outcomes.${reason}`, {
                    defaultValue: t("dictation.timing.cleanupFallbackUnknown"),
                  }),
                  count,
                })}
              </li>
            ))}
          </ul>
        </div>
      )}
      {summary.slowest.length > 0 && (
        <div className="space-y-2">
          <h3 className="text-sm font-medium">
            {t("dictation.timing.slowerTitle")}
          </h3>
          {summary.slowest.map((timing) => (
            <details
              key={timing.id}
              className="rounded border border-mid-gray/20 p-2 text-xs"
            >
              <summary className="cursor-pointer text-text/65">
                {t("dictation.timing.slowerRecord", {
                  date: new Date(timing.recorded_at_ms).toLocaleString(
                    i18n.language,
                    {
                      month: "numeric",
                      day: "numeric",
                      hour: "2-digit",
                      minute: "2-digit",
                    },
                  ),
                  value: seconds(timing.total_ms),
                })}
              </summary>
              <div className="pt-3">
                <DictationTimingDetails timing={timing} />
              </div>
            </details>
          ))}
        </div>
      )}
    </div>
  );
};

export const LatestDictationTiming = () => {
  const { t } = useTranslation();
  const {
    timing: { metrics, failed, loading, retry },
  } = useDictationData();
  return (
    <SettingsGroup title={t("dictation.timing.summaryTitle")}>
      <div className="p-4">
        {loading ? (
          <p className="text-sm text-text/55">{t("common.loading")}</p>
        ) : failed ? (
          <div className="flex gap-3 items-center justify-between">
            <p role="alert" className="text-sm text-text/65">
              {t("dictation.timing.loadFailed")}
            </p>
            <Button variant="secondary" onClick={retry}>
              {t("dictation.retry")}
            </Button>
          </div>
        ) : metrics[0] ? (
          <div className="space-y-5">
            <DictationTimingSummary metrics={metrics} />
            <div className="space-y-2 border-t border-mid-gray/20 pt-4">
              <h3 className="text-sm font-medium">
                {t("dictation.timing.title")}
              </h3>
              <DictationTimingDetails timing={metrics[0]} />
            </div>
          </div>
        ) : (
          <p className="text-sm text-text/55">{t("dictation.timing.empty")}</p>
        )}
      </div>
    </SettingsGroup>
  );
};
