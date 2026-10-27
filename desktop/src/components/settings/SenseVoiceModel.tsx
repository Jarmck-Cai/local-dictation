import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Cpu, Loader2 } from "lucide-react";
import { toast } from "sonner";
import { useModelStore } from "@/stores/modelStore";
import { RECOGNITION_MODEL_ID } from "@/lib/dictation";
import DownloadProgressDisplay from "../model-selector/DownloadProgressDisplay";
import { Button } from "../ui/Button";
import { SettingsGroup } from "../ui/SettingsGroup";

export const SenseVoiceModel = () => {
  const { t } = useTranslation();
  const [switching, setSwitching] = useState(false);
  const store = useModelStore();
  const model = store.models.find((entry) => entry.id === RECOGNITION_MODEL_ID);
  const modelId = model?.id ?? RECOGNITION_MODEL_ID;
  const downloading = modelId in store.downloadingModels;
  const verifying = modelId in store.verifyingModels;
  const extracting = modelId in store.extractingModels;
  const active = store.currentModel === modelId && model?.is_downloaded;

  const activate = async () => {
    setSwitching(true);
    try {
      if (!(await store.selectModel(modelId))) {
        toast.error(t("dictation.model.actionFailed"));
      }
    } finally {
      setSwitching(false);
    }
  };

  const download = async () => {
    if (!(await store.downloadModel(modelId))) {
      toast.error(t("dictation.model.actionFailed"));
    }
  };

  const working = switching || verifying || extracting;
  const status = switching
    ? "activating"
    : extracting
      ? "extracting"
      : verifying
        ? "verifying"
        : downloading
          ? "downloading"
          : active
            ? "selected"
            : model?.is_downloaded
              ? "available"
              : "needed";

  return (
    <SettingsGroup title={t("dictation.model.title")}>
      <div className="p-4 space-y-3">
        <div className="flex items-center justify-between gap-3">
          <div className="flex items-center gap-3">
            <Cpu size={22} className="text-background-ui" aria-hidden="true" />
            <div>
              <h3 className="font-semibold">{t("dictation.model.name")}</h3>
              <p className="text-xs text-text/60">
                {t("dictation.model.description")}
              </p>
            </div>
          </div>
          <div className="flex items-center gap-2">
            {working && (
              <Loader2 size={16} className="animate-spin" aria-hidden="true" />
            )}
            <span className="text-xs text-text/65">
              {t(`dictation.model.states.${status}`)}
            </span>
            {model && !model.is_downloaded && !downloading && !working && (
              <Button onClick={download} disabled={store.loading}>
                {t("dictation.model.download")}
              </Button>
            )}
            {model?.is_downloaded && !active && !working && (
              <Button onClick={activate}>
                {t("dictation.model.activate")}
              </Button>
            )}
            {downloading && (
              <Button
                variant="secondary"
                onClick={async () => {
                  if (!(await store.cancelDownload(modelId)))
                    toast.error(t("dictation.model.actionFailed"));
                }}
              >
                {t("modelSelector.cancel")}
              </Button>
            )}
          </div>
        </div>
        {downloading && (
          <DownloadProgressDisplay
            downloadProgress={
              store.downloadProgress[modelId]
                ? { [modelId]: store.downloadProgress[modelId] }
                : {}
            }
            downloadStats={store.downloadStats}
          />
        )}
        {!store.loading && !model && (
          <p role="alert" className="text-sm text-error">
            {t("dictation.model.unavailable")}
          </p>
        )}
        <p className="text-xs text-text/55">
          {t("dictation.model.notBenchmarked")}
        </p>
      </div>
    </SettingsGroup>
  );
};
