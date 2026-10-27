import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { toast, Toaster } from "sonner";
import { useTranslation } from "react-i18next";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import "./App.css";
import {
  Sidebar,
  type SidebarSection,
  SECTIONS_CONFIG,
} from "./components/Sidebar";
import { Button } from "./components/ui/Button";
import { useSettings } from "./hooks/useSettings";
import { commands } from "@/bindings";
import type { ModelStateEvent, RecordingErrorEvent } from "./lib/types/events";
import { getLanguageDirection, initializeRTL } from "@/lib/utils/rtl";

function App() {
  const { t, i18n } = useTranslation();
  const [currentSection, setCurrentSection] =
    useState<SidebarSection>("general");
  const [microphoneDenied, setMicrophoneDenied] = useState(false);
  const {
    settings,
    isLoading,
    refreshSettings,
    refreshAudioDevices,
    refreshOutputDevices,
  } = useSettings();
  const initialized = useRef(false);
  const scrollRef = useRef<HTMLDivElement>(null);

  useLayoutEffect(() => {
    scrollRef.current?.scrollTo({ top: 0 });
  }, [currentSection]);
  useEffect(() => {
    initializeRTL(i18n.language);
  }, [i18n.language]);

  const checkMicrophone = useCallback(async () => {
    try {
      const status = await commands.getWindowsMicrophonePermissionStatus();
      setMicrophoneDenied(
        status.supported && status.overall_access === "denied",
      );
    } catch {
      // Device errors still surface through recording-error if this OS query is unavailable.
    }
  }, []);

  useEffect(() => {
    if (!settings || initialized.current) return;
    initialized.current = true;
    void Promise.all([
      commands.initializeEnigo(),
      commands.initializeShortcuts(),
    ])
      .then((results) => {
        if (results.some((result) => result.status !== "ok"))
          toast.error(t("dictation.initializationFailed"));
      })
      .catch(() => toast.error(t("dictation.initializationFailed")));
    void refreshAudioDevices();
    void refreshOutputDevices();
    void checkMicrophone();
  }, [settings, refreshAudioDevices, refreshOutputDevices, checkMicrophone, t]);

  useEffect(() => {
    let disposed = false;
    const unlisteners: UnlistenFn[] = [];
    const register = async <T,>(
      name: string,
      callback: (payload: T) => void,
    ) => {
      const stop = await listen<T>(name, (event) => callback(event.payload));
      if (disposed) stop();
      else unlisteners.push(stop);
    };
    void register<RecordingErrorEvent>("recording-error", (event) => {
      if (event.error_type === "microphone_permission_denied") {
        setMicrophoneDenied(true);
        toast.error(t("errors.micPermissionDeniedTitle"), {
          description: t("errors.micPermissionDenied.windows"),
        });
      } else if (event.error_type === "no_input_device") {
        toast.error(t("errors.noInputDeviceTitle"), {
          description: t("errors.noInputDevice"),
        });
      } else {
        toast.error(t("dictation.recordingFailed"));
      }
    });
    void register("paste-error", () => {
      toast.error(t("errors.pasteFailedTitle"), {
        description: t("errors.pasteFailed"),
      });
    });
    void register("transcription-error", () => {
      toast.error(t("errors.transcriptionFailedTitle"), {
        description: t("dictation.transcriptionFailed"),
      });
    });
    void register("playback-mute-error", () => {
      toast.error(t("dictation.muteFailed"));
    });
    void register<boolean>("dictation-copy-result", (copied) => {
      if (copied) toast.success(t("dictation.history.copied"));
      else toast.error(t("dictation.history.copyLatestFailed"));
    });
    void register<ModelStateEvent>("model-state-changed", (event) => {
      if (event.event_type === "loading_failed")
        toast.error(t("dictation.model.actionFailed"));
    });
    return () => {
      disposed = true;
      unlisteners.forEach((stop) => stop());
    };
  }, [t]);

  // Mount settings controls only after the shared store finishes its first read.
  // Each control uses the same hook; mounting them sooner would repeat startup I/O.
  if (isLoading || !settings) {
    return (
      <div className="h-screen flex flex-col items-center justify-center gap-4">
        <p role="status" className="text-sm text-text/60">
          {t(isLoading ? "common.loading" : "dictation.settingsFailed")}
        </p>
        {!isLoading && (
          <Button onClick={refreshSettings} variant="secondary">
            {t("dictation.retry")}
          </Button>
        )}
      </div>
    );
  }

  return (
    <div
      dir={getLanguageDirection(i18n.language)}
      className="h-screen flex select-none cursor-default overflow-hidden"
    >
      <Toaster theme="system" richColors closeButton />
      <Sidebar
        activeSection={currentSection}
        onSectionChange={setCurrentSection}
      />
      <main className="flex-1 min-w-0 flex flex-col overflow-hidden">
        <header className="shrink-0 px-7 py-5 border-b border-mid-gray/15">
          <h1 className="font-semibold text-xl">
            {t(SECTIONS_CONFIG[currentSection].labelKey)}
          </h1>
          <p className="mt-1 text-xs text-text/50">{t("dictation.tagline")}</p>
        </header>
        <div ref={scrollRef} className="flex-1 overflow-y-auto p-6">
          {microphoneDenied && (
            <div className="max-w-3xl mx-auto mb-5 rounded-lg border border-warning/25 bg-warning/10 p-4 space-y-3">
              <p className="text-sm">
                {t("errors.micPermissionDenied.windows")}
              </p>
              <div className="flex gap-2">
                <Button
                  variant="secondary"
                  onClick={() => commands.openMicrophonePrivacySettings()}
                >
                  {t("dictation.openMicrophoneSettings")}
                </Button>
                <Button
                  variant="secondary"
                  onClick={async () => {
                    await checkMicrophone();
                    await refreshAudioDevices();
                  }}
                >
                  {t("dictation.retry")}
                </Button>
              </div>
            </div>
          )}
          {Object.entries(SECTIONS_CONFIG).map(([id, section]) => {
            const Section = section.component;
            return (
              <div key={id} hidden={currentSection !== id}>
                <Section />
              </div>
            );
          })}
        </div>
      </main>
    </div>
  );
}

export default App;
