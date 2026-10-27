import { useTranslation } from "react-i18next";
import { MicrophoneSelector } from "../MicrophoneSelector";
import { ShortcutInput } from "../ShortcutInput";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { MuteWhileRecording } from "../MuteWhileRecording";
import { DictationCleanup } from "../DictationCleanup";
import { SenseVoiceModel } from "../SenseVoiceModel";
import { AutostartToggle } from "../AutostartToggle";
import { ManualPasteShortcut } from "../ManualPasteShortcut";
import { ClipboardHandlingSetting } from "../ClipboardHandling";
import { PasteMethodSetting } from "../PasteMethod";
import { AppLanguageSelector } from "../AppLanguageSelector";
import { ThemeSelector } from "../ThemeSelector";

export const GeneralSettings = () => {
  const { t } = useTranslation();
  return (
    <div className="qs-page space-y-6">
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
        <ManualPasteShortcut />
        <ShortcutInput shortcutId="toggle_cloud" grouped />
        <MicrophoneSelector grouped />
        <MuteWhileRecording grouped />
        <AutostartToggle descriptionMode="inline" grouped />
      </SettingsGroup>
      <SettingsGroup title={t("dictation.preferencesTitle")}>
        <PasteMethodSetting descriptionMode="inline" grouped />
        <ClipboardHandlingSetting descriptionMode="inline" grouped />
        <AppLanguageSelector grouped />
        <ThemeSelector grouped />
      </SettingsGroup>
      <p className="px-4 text-xs text-text/50">{t("dictation.retention")}</p>
    </div>
  );
};
