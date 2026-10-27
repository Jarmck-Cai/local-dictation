import { invoke } from "@tauri-apps/api/core";

export const RECOGNITION_MODEL_ID = "sense-voice-int8";

export type CleanupMode = "claude" | "local" | "off";

export interface DictationConfig {
  mode: CleanupMode;
  local_base_url: string;
  local_model: string;
  timeout_ms: number;
}

// Credentials never cross this read interface. Only a configuration flag does.
export interface DictationConfigView extends DictationConfig {
  claude_key_configured: boolean;
}

export interface CleanupStatus {
  stage: "cleaning" | "ready" | "fallback";
  reason?: string;
}

export const dictationCommands = {
  getConfig: () => invoke<DictationConfigView>("get_dictation_config"),
  saveConfig: (config: DictationConfig) =>
    invoke<DictationConfigView>("save_dictation_config", { config }),
  setClaudeKey: (key: string) => invoke<void>("set_claude_api_key", { key }),
  clearClaudeKey: () => invoke<void>("clear_claude_api_key"),
};
