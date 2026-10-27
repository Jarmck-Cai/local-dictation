import { useTranslation } from "react-i18next";
import { useDictationMetrics } from "@/hooks/useDictationMetrics";
import type { DictationTiming } from "@/lib/dictationApi";
import { distinctCleanupReason } from "@/lib/dictationState";
import { Button } from "../ui/Button";
import { SettingsGroup } from "../ui/SettingsGroup";

export const DictationTimingDetails = ({
  timing,
}: {
  timing: DictationTiming;
}) => {
  const { t, i18n } = useTranslation();
  const cleanupReason = distinctCleanupReason(timing);
  const seconds = (milliseconds: number) =>
    t("dictation.timing.seconds", {
      value: (milliseconds / 1000).toLocaleString(i18n.language, {
        maximumFractionDigits: 2,
      }),
    });
  const values = [
    ["asr", seconds(timing.asr_ms)],
    ["cleanup", seconds(timing.cleanup_ms)],
    [
      "paste",
      timing.paste_ms === null
        ? t("dictation.timing.unavailable")
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
      <p className="text-text/45">{t("dictation.timing.note")}</p>
    </div>
  );
};

export const LatestDictationTiming = () => {
  const { t } = useTranslation();
  const { metrics, failed, retry } = useDictationMetrics();
  return (
    <SettingsGroup title={t("dictation.timing.title")}>
      <div className="p-4">
        {failed ? (
          <div className="flex gap-3 items-center justify-between">
            <p role="alert" className="text-sm text-text/65">
              {t("dictation.timing.loadFailed")}
            </p>
            <Button variant="secondary" onClick={retry}>
              {t("dictation.retry")}
            </Button>
          </div>
        ) : metrics[0] ? (
          <DictationTimingDetails timing={metrics[0]} />
        ) : (
          <p className="text-sm text-text/55">{t("dictation.timing.empty")}</p>
        )}
      </div>
    </SettingsGroup>
  );
};
