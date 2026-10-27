import { useEffect, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import { Mic, Loader2 } from "lucide-react";
import { useSettings } from "@/hooks/useSettings";
import { useOsType } from "@/hooks/useOsType";
import { formatKeyCombination } from "@/lib/utils/keyboard";
import { useModelStore } from "@/stores/modelStore";
import { RECOGNITION_MODEL_ID, type CleanupStatus } from "@/lib/dictation";

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
  const { models, currentModel } = useModelStore();
  const available =
    currentModel === RECOGNITION_MODEL_ID &&
    models.some((model) => model.id === currentModel && model.is_downloaded);

  useEffect(() => {
    let disposed = false;
    const unlisteners: UnlistenFn[] = [];
    const register = async <T,>(
      name: string,
      callback: (payload: T) => void,
    ) => {
      const unlisten = await listen<T>(name, (event) =>
        callback(event.payload),
      );
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
    return () => {
      disposed = true;
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

  return (
    <section className="rounded-xl bg-logo-primary/10 border border-logo-primary/25 p-5 space-y-3">
      <div className="flex items-center gap-3" role="status" aria-live="polite">
        <span className="flex items-center justify-center rounded-xl w-11 h-11 bg-logo-primary/20 text-background-ui">
          {busy ? (
            <Loader2 size={23} className="animate-spin" aria-hidden="true" />
          ) : (
            <Mic size={23} aria-hidden="true" />
          )}
        </span>
        <div>
          <h2 className="text-lg font-semibold">
            {t(`dictation.status.${status}`)}
          </h2>
          <p className="text-sm text-text/65">
            {t("dictation.status.shortcutHint", { shortcut, cancel })}
          </p>
        </div>
      </div>
      <p className="text-xs text-text/60">
        {t("dictation.status.currentCursor")}
      </p>
      {fallback && (
        <p role="status" className="text-sm text-warning">
          {t("dictation.status.fallback")}
        </p>
      )}
    </section>
  );
};
