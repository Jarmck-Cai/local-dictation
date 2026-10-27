import { useEffect, useId, useRef, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import { Info, Loader2 } from "lucide-react";
import { useSettings } from "@/hooks/useSettings";
import { useOsType } from "@/hooks/useOsType";
import { formatKeyCombination } from "@/lib/utils/keyboard";
import { useModelStore } from "@/stores/modelStore";
import {
  RECOGNITION_MODEL_ID,
  DICTATION_CONFIG_REFRESH,
  dictationCommands,
  type CleanupStatus,
  type DictationConfigView,
} from "@/lib/dictation";
import { dictationApi, type CloudStatus } from "@/lib/dictationApi";
import { SnapshotGuard } from "@/lib/dictationState";

type DictationState =
  | "idle"
  | "recording"
  | "streaming"
  | "transcribing"
  | "processing";

export const DictationStatus = () => {
  const { t } = useTranslation();
  const { settings } = useSettings();
  const osType = useOsType();
  const [state, setState] = useState<DictationState>("idle");
  const [fallback, setFallback] = useState(false);
  const [config, setConfig] = useState<DictationConfigView | null>(null);
  const [cloud, setCloud] = useState<CloudStatus | null>(null);
  const [showHelp, setShowHelp] = useState(false);
  const helpId = useId();
  const configGeneration = useRef(0);
  const { models, currentModel } = useModelStore();
  const available =
    currentModel === RECOGNITION_MODEL_ID &&
    models.some((model) => model.id === currentModel && model.is_downloaded);

  useEffect(() => {
    let disposed = false;
    const load = async () => {
      const generation = ++configGeneration.current;
      try {
        const view = await dictationCommands.getConfig();
        if (!disposed && generation === configGeneration.current)
          setConfig(view);
      } catch {
        if (!disposed && generation === configGeneration.current)
          setConfig(null);
      }
    };
    void load();
    window.addEventListener(DICTATION_CONFIG_REFRESH, load);
    return () => {
      disposed = true;
      window.removeEventListener(DICTATION_CONFIG_REFRESH, load);
    };
  }, []);

  useEffect(() => {
    let disposed = false;
    const unlisteners: UnlistenFn[] = [];
    const guard = new SnapshotGuard();
    const register = async <T,>(
      name: string,
      callback: (payload: T) => void,
    ) => {
      const unlisten = await listen<T>(name, (event) => {
        if (!disposed) callback(event.payload);
      });
      if (disposed) unlisten();
      else unlisteners.push(unlisten);
    };
    void register<DictationState>("dictation-state", (next) => {
      setState(next);
      if (next === "recording" || next === "streaming") setFallback(false);
    });
    void register<CleanupStatus>("dictation-cleanup-status", (status) => {
      if (status.stage === "cleaning") setState("processing");
      if (status.stage === "fallback") setFallback(true);
    });
    void (async () => {
      try {
        await register<CloudStatus>("dictation-cloud-state", (payload) => {
          guard.invalidate();
          setCloud(payload);
        });
        if (disposed) return;
        const ticket = guard.begin();
        const snapshot = await dictationApi.getCloudStatus();
        if (guard.isCurrent(ticket)) setCloud(snapshot);
      } catch {
        /* Configuration and cloud controls provide their own retry UI. */
      }
    })();
    return () => {
      disposed = true;
      guard.dispose();
      unlisteners.forEach((unlisten) => unlisten());
    };
  }, []);

  const recording = state === "recording" || state === "streaming";
  const busy = state === "transcribing" || state === "processing";
  const status = recording
    ? "recording"
    : busy
      ? state
      : available
        ? "ready"
        : "setup";
  const shortcut = formatKeyCombination(
    settings?.bindings?.transcribe?.current_binding ?? "F8",
    osType,
  );
  const cancel = formatKeyCombination(
    settings?.bindings?.cancel?.current_binding ?? "Esc",
    osType,
  );
  const paused =
    cloud &&
    (cloud.manual_paused ||
      cloud.cooldown_remaining_ms > 0 ||
      cloud.consecutive_failures >= 3);
  const modeLabel = !config
    ? t("dictation.footer.modeUnavailable")
    : config.mode === "claude" && paused
      ? t("dictation.footer.cloudPaused")
      : config.mode === "off"
        ? t("dictation.cleanup.hints.off")
        : t("dictation.footer.mode", {
            mode: t(`dictation.cleanup.modes.${config.mode}`),
            strength: t(`dictation.cleanup.strengths.${config.strength}`),
          });

  return (
    <footer className="qs-status-footer">
      <div className="qs-status-line">
        <div className="qs-status-mode" role="status" aria-live="polite">
          {busy ? (
            <Loader2 size={13} className="animate-spin" aria-hidden="true" />
          ) : (
            <span
              className={`qs-status-dot ${recording ? "qs-status-dot-recording" : ""}`}
              aria-hidden="true"
            />
          )}
          <span>{t(`dictation.status.${status}`)}</span>
          <span className="text-text/30" aria-hidden="true">
            {t("dictation.footer.separator")}
          </span>
          <span>{modeLabel}</span>
          <button
            type="button"
            className="qs-info"
            aria-label={t("dictation.footer.dataHelp")}
            aria-expanded={showHelp}
            aria-controls={helpId}
            onClick={() => setShowHelp((value) => !value)}
          >
            <Info size={14} aria-hidden="true" />
          </button>
        </div>
        <p className="qs-shortcut">
          {t("dictation.footer.shortcut", { shortcut })}
        </p>
      </div>
      {fallback && (
        <p role="status" className="mt-1 text-xs text-warning">
          {t("dictation.status.fallback")}
        </p>
      )}
      <div id={helpId} hidden={!showHelp} className="qs-data-help space-y-1.5">
        <h2 className="font-medium text-text">
          {t("dictation.footer.dataTitle")}
        </h2>
        <p>{t("dictation.footer.audioLocal")}</p>
        <p>
          {config
            ? t(`dictation.footer.dataModes.${config.mode}`)
            : t("dictation.footer.modeUnavailable")}
        </p>
        <p>{t("dictation.retention")}</p>
        <p>{t("dictation.status.currentCursor")}</p>
        <p>{t("dictation.status.shortcutHint", { shortcut, cancel })}</p>
      </div>
    </footer>
  );
};
