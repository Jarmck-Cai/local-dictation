import { useTranslation } from "react-i18next";
import { MicrophoneSelector } from "../MicrophoneSelector";
import { ShortcutInput } from "../ShortcutInput";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { MuteWhileRecording } from "../MuteWhileRecording";
import {
  ClaudeKeySettings,
  DictationCleanup,
  useDictationCleanupSettings,
} from "../DictationCleanup";
import { SenseVoiceModel } from "../SenseVoiceModel";
import {
  CloudGatewaySettings,
  RecognitionModeSettings,
  useCloudGatewaySettings,
} from "../CloudGatewaySettings";
import { AutostartToggle } from "../AutostartToggle";
import { ManualPasteShortcut } from "../ManualPasteShortcut";
import { ClipboardHandlingSetting } from "../ClipboardHandling";
import { PasteMethodSetting } from "../PasteMethod";
import { AppLanguageSelector } from "../AppLanguageSelector";
import { ThemeSelector } from "../ThemeSelector";

export const GeneralSettings = () => {
  const { t } = useTranslation();
  const gateway = useCloudGatewaySettings();
  const cleanup = useDictationCleanupSettings();
  return (
    <div className="qs-page space-y-5">
      <SettingsGroup title={t("dictation.settingsTitle")}>
        <ShortcutInput
          shortcutId="transcribe"
          descriptionMode="inline"
          grouped
        />
        <MicrophoneSelector grouped />
      </SettingsGroup>
      <RecognitionModeSettings settings={gateway} />
      <DictationCleanup settings={cleanup} />
      <CloudGatewaySettings settings={gateway}>
        <ClaudeKeySettings settings={cleanup} />
      </CloudGatewaySettings>
      <SenseVoiceModel />
      <SettingsGroup title={t("settings.general.shortcut.title")}>
        <ShortcutInput shortcutId="cancel" grouped />
        <ShortcutInput shortcutId="copy_latest_result" grouped />
        <ManualPasteShortcut />
        <ShortcutInput shortcutId="toggle_cloud" grouped />
      </SettingsGroup>
      <SettingsGroup title={t("dictation.preferencesTitle")}>
        <MuteWhileRecording grouped />
        <AutostartToggle descriptionMode="inline" grouped />
        <PasteMethodSetting descriptionMode="inline" grouped />
        <ClipboardHandlingSetting descriptionMode="inline" grouped />
        <AppLanguageSelector grouped />
        <ThemeSelector grouped />
      </SettingsGroup>
      <p className="px-4 text-xs text-text/50">{t("dictation.retention")}</p>
    </div>
  );
};
