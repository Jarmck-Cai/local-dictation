import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
  dictationCommands,
  type CleanupMode,
  type DictationConfigView,
} from "@/lib/dictation";
import { Button } from "../ui/Button";
import { Input } from "../ui/Input";
import { SettingsGroup } from "../ui/SettingsGroup";

export const DictationCleanup = () => {
  const { t } = useTranslation();
  const [config, setConfig] = useState<DictationConfigView | null>(null);
  const [busy, setBusy] = useState(false);
  const [loadFailed, setLoadFailed] = useState(false);
  const [key, setKey] = useState("");
  const [dirty, setDirty] = useState(false);

  const load = useCallback(async () => {
    setBusy(true);
    setLoadFailed(false);
    try {
      setConfig(await dictationCommands.getConfig());
      setDirty(false);
    } catch {
      setLoadFailed(true);
    } finally {
      setBusy(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const save = async () => {
    if (!config) return;
    setBusy(true);
    try {
      const { mode, local_base_url, local_model, timeout_ms } = config;
      setConfig(
        await dictationCommands.saveConfig({
          mode,
          local_base_url: local_base_url.trim(),
          local_model: local_model.trim(),
          timeout_ms,
        }),
      );
      setDirty(false);
      toast.success(t("dictation.cleanup.saved"));
    } catch {
      toast.error(t("dictation.cleanup.saveFailed"));
    } finally {
      setBusy(false);
    }
  };

  const changeKey = async (clear: boolean) => {
    setBusy(true);
    try {
      if (clear) await dictationCommands.clearClaudeKey();
      else await dictationCommands.setClaudeKey(key.trim());
      const view = await dictationCommands.getConfig();
      setConfig((previous) =>
        previous
          ? { ...previous, claude_key_configured: view.claude_key_configured }
          : view,
      );
      toast.success(
        t(
          clear ? "dictation.cleanup.keyRemoved" : "dictation.cleanup.keySaved",
        ),
      );
    } catch {
      toast.error(t("dictation.cleanup.keyFailed"));
    } finally {
      setKey("");
      setBusy(false);
    }
  };

  return (
    <SettingsGroup title={t("dictation.cleanup.title")}>
      <div className="p-4 space-y-4">
        <p className="text-sm text-text/65">
          {t("dictation.cleanup.description")}
        </p>
        {loadFailed ? (
          <div className="flex items-center justify-between gap-3">
            <p role="alert" className="text-sm text-error">
              {t("dictation.cleanup.loadFailed")}
            </p>
            <Button variant="secondary" onClick={load} disabled={busy}>
              {t("dictation.retry")}
            </Button>
          </div>
        ) : !config ? (
          <p className="text-sm text-text/60">{t("common.loading")}</p>
        ) : (
          <>
            <div
              className="grid grid-cols-3 gap-2"
              role="group"
              aria-label={t("dictation.cleanup.mode")}
            >
              {(["claude", "local", "off"] as const).map(
                (mode: CleanupMode) => (
                  <Button
                    key={mode}
                    variant={
                      config.mode === mode ? "primary-soft" : "secondary"
                    }
                    aria-pressed={config.mode === mode}
                    disabled={busy}
                    onClick={() => {
                      setConfig({ ...config, mode });
                      setDirty(true);
                    }}
                  >
                    {t(`dictation.cleanup.modes.${mode}`)}
                  </Button>
                ),
              )}
            </div>
            <p className="text-xs text-text/65">
              {t(`dictation.cleanup.hints.${config.mode}`)}
            </p>
            {config.mode === "claude" && (
              <div className="space-y-3">
                <p className="text-sm">
                  {t(
                    config.claude_key_configured
                      ? "dictation.cleanup.keyConfigured"
                      : "dictation.cleanup.keyMissing",
                  )}
                </p>
                <div className="flex flex-wrap items-center gap-2">
                  <Input
                    type="password"
                    autoComplete="new-password"
                    spellCheck={false}
                    value={key}
                    onChange={(event) => setKey(event.target.value)}
                    disabled={busy}
                    aria-label={t("dictation.cleanup.keyLabel")}
                    placeholder={t("dictation.cleanup.keyPlaceholder")}
                    className="min-w-48 flex-1"
                  />
                  <Button
                    variant="secondary"
                    disabled={busy || !key.trim()}
                    onClick={() => changeKey(false)}
                  >
                    {t("dictation.cleanup.saveKey")}
                  </Button>
                  {config.claude_key_configured && (
                    <Button
                      variant="danger-ghost"
                      disabled={busy}
                      onClick={() => changeKey(true)}
                    >
                      {t("dictation.cleanup.removeKey")}
                    </Button>
                  )}
                </div>
              </div>
            )}
            {config.mode === "local" && (
              <div className="grid gap-3">
                <label className="grid gap-1.5 text-sm">
                  {t("dictation.cleanup.localAddress")}
                  <Input
                    value={config.local_base_url}
                    disabled={busy}
                    spellCheck={false}
                    placeholder={t("dictation.cleanup.localAddressPlaceholder")}
                    onChange={(event) => {
                      setConfig({
                        ...config,
                        local_base_url: event.target.value,
                      });
                      setDirty(true);
                    }}
                  />
                </label>
                <label className="grid gap-1.5 text-sm">
                  {t("dictation.cleanup.localModel")}
                  <Input
                    value={config.local_model}
                    disabled={busy}
                    spellCheck={false}
                    placeholder={t("dictation.cleanup.localModelPlaceholder")}
                    onChange={(event) => {
                      setConfig({ ...config, local_model: event.target.value });
                      setDirty(true);
                    }}
                  />
                </label>
              </div>
            )}
            <div className="flex items-center justify-between gap-3">
              <p className="text-xs text-text/55">
                {t("dictation.cleanup.fallback")}
              </p>
              <Button disabled={busy || !dirty} onClick={save}>
                {t("dictation.cleanup.save")}
              </Button>
            </div>
          </>
        )}
      </div>
    </SettingsGroup>
  );
};
