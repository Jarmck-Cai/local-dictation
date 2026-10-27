import { useTranslation } from "react-i18next";
import { MicrophoneSelector } from "../MicrophoneSelector";
import { ShortcutInput } from "../ShortcutInput";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { MuteWhileRecording } from "../MuteWhileRecording";
import { DictationCleanup } from "../DictationCleanup";
import { DictationStatus } from "../DictationStatus";
import { SenseVoiceModel } from "../SenseVoiceModel";
import { HistorySettings } from "../history/HistorySettings";
import { LatestDictationTiming } from "../DictationTiming";

export const GeneralSettings = () => {
  const { t } = useTranslation();
  return (
    <div className="max-w-3xl w-full mx-auto space-y-6">
      <DictationStatus />
      <HistorySettings latestOnly />
      <LatestDictationTiming />
      <SenseVoiceModel />
      <DictationCleanup />
      <SettingsGroup title={t("dictation.settingsTitle")}>
        <ShortcutInput
          shortcutId="transcribe"
          descriptionMode="inline"
          grouped
        />
        <ShortcutInput shortcutId="cancel" grouped />
        <ShortcutInput shortcutId="copy_latest_result" grouped />
        <ShortcutInput shortcutId="toggle_cloud" grouped />
        <MicrophoneSelector grouped />
        <MuteWhileRecording grouped />
      </SettingsGroup>
      <p className="px-4 text-xs text-text/50">{t("dictation.retention")}</p>
    </div>
  );
};
