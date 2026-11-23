import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { useOsType } from "@/hooks/useOsType";
import { useTranslation } from "react-i18next";
import { Cloud, RefreshCw, ShieldCheck } from "lucide-react";
import { toast } from "sonner";
import {
  cloudGatewayCommands,
  classifyGatewayErrorCode,
  gatewayEffectiveBannerKey,
  isValidGatewayUrlInput,
  normalizeGatewayUrlInput,
  type CloudGatewayView,
  type GatewayConnectionTestResult,
  type RecognitionMode,
} from "@/lib/cloudGateway";
import { Button } from "../ui/Button";
import { Input } from "../ui/Input";
import { SettingsGroup } from "../ui/SettingsGroup";

export const CloudGatewaySettings = () => {
  const { t } = useTranslation();
  const isMac = useOsType() === "macos";
  const [view, setView] = useState<CloudGatewayView | null>(null);
  const [busy, setBusy] = useState(false);
  const [testing, setTesting] = useState(false);
  const [syncing, setSyncing] = useState(false);
  const [loadFailed, setLoadFailed] = useState(false);
  const [tokenInput, setTokenInput] = useState("");
  const [dirty, setDirty] = useState(false);
  const [testResult, setTestResult] =
    useState<GatewayConnectionTestResult | null>(null);

  const load = useCallback(async () => {
    setBusy(true);
    setLoadFailed(false);
    try {
      setView(await cloudGatewayCommands.getConfig());
      setDirty(false);
    } catch {
      setLoadFailed(true);
    } finally {
      setBusy(false);
    }
  }, []);

  useEffect(() => {
    void load();
    let active = true;
    let unlisten: (() => void) | undefined;
    void listen<CloudGatewayView>("cloud-gateway-updated", (event) => {
      if (active) {
        setView((prev) =>
          dirty && prev
            ? {
                ...event.payload,
                recognition_mode: prev.recognition_mode,
                gateway_url: prev.gateway_url,
                auto_sync_vocabulary: prev.auto_sync_vocabulary,
              }
            : event.payload,
        );
      }
    }).then((fn) => {
      if (!active) fn();
      else unlisten = fn;
    });
    return () => {
      active = false;
      unlisten?.();
    };
  }, [load, dirty]);

  const saveConfig = async () => {
    if (!view) return;
    if (!isValidGatewayUrlInput(view.gateway_url)) {
      toast.error(t("dictation.cloudGateway.errors.invalid_gateway_url"));
      return;
    }
    setBusy(true);
    try {
      const updated = await cloudGatewayCommands.saveConfig({
        recognition_mode: view.recognition_mode,
        gateway_url: normalizeGatewayUrlInput(view.gateway_url),
        auto_sync_vocabulary: view.auto_sync_vocabulary,
      });
      setView(updated);
      setDirty(false);
      toast.success(t("dictation.cloudGateway.saved"));
    } catch (err) {
      const code = classifyGatewayErrorCode(err);
      toast.error(t(`dictation.cloudGateway.errors.${code}`));
    } finally {
      setBusy(false);
    }
  };

  const updateToken = async (clear: boolean) => {
    setBusy(true);
    try {
      const updated = clear
        ? await cloudGatewayCommands.clearToken()
        : await cloudGatewayCommands.setToken(tokenInput.trim());
      setView((prev) =>
        prev
          ? {
              ...updated,
              recognition_mode: prev.recognition_mode,
              gateway_url: prev.gateway_url,
              auto_sync_vocabulary: prev.auto_sync_vocabulary,
            }
          : updated,
      );
      setTestResult(null);
      toast.success(
        t(
          clear
            ? "dictation.cloudGateway.tokenRemoved"
            : "dictation.cloudGateway.tokenSaved",
        ),
      );
    } catch (err) {
      const code = classifyGatewayErrorCode(err);
      toast.error(t(`dictation.cloudGateway.errors.${code}`));
    } finally {
      setTokenInput("");
      setBusy(false);
    }
  };

  const runTest = async () => {
    if (!view) return;
    const cleanedUrl = normalizeGatewayUrlInput(view.gateway_url);
    if (!cleanedUrl || !isValidGatewayUrlInput(cleanedUrl)) {
      toast.error(t("dictation.cloudGateway.errors.invalid_gateway_url"));
      return;
    }
    setTesting(true);
    setTestResult(null);
    try {
      const result = await cloudGatewayCommands.testConnection(cleanedUrl);
      setTestResult(result);
      if (result.authenticated && result.asr_ready) {
        toast.success(
          t("dictation.cloudGateway.testSuccess", {
            ms: result.latency_ms,
          }),
        );
      } else if (!result.authenticated) {
        toast.error(t("dictation.cloudGateway.errors.gateway_unauthorized"));
      }
    } catch (err) {
      const code = classifyGatewayErrorCode(err);
      toast.error(t(`dictation.cloudGateway.errors.${code}`));
    } finally {
      setTesting(false);
    }
  };

  const runSync = async (fullSync: boolean) => {
    setSyncing(true);
    try {
      if (dirty && view) {
        const saved = await cloudGatewayCommands.saveConfig({
          recognition_mode: view.recognition_mode,
          gateway_url: normalizeGatewayUrlInput(view.gateway_url),
          auto_sync_vocabulary: view.auto_sync_vocabulary,
        });
        setView(saved);
        setDirty(false);
      }
      const res = await cloudGatewayCommands.syncVocabulary(fullSync);
      const refreshed = await cloudGatewayCommands.getConfig();
      setView(refreshed);
      toast.success(
        t("dictation.cloudGateway.syncSuccess", {
          active: res.total_active,
          revision: res.revision,
        }),
      );
    } catch (err) {
      const code = classifyGatewayErrorCode(err);
      toast.error(t(`dictation.cloudGateway.errors.${code}`));
    } finally {
      setSyncing(false);
    }
  };

  const bannerKey = view ? gatewayEffectiveBannerKey(view) : null;

  return (
    <SettingsGroup title={t("dictation.cloudGateway.title")}>
      <div className="p-4 space-y-4">
        <div className="flex items-start gap-3">
          <Cloud
            size={20}
            className="mt-0.5 shrink-0 text-logo-primary"
            aria-hidden="true"
          />
          <p className="text-sm text-text/65">
            {t("dictation.cloudGateway.description")}
          </p>
        </div>
        {loadFailed ? (
          <div className="flex items-center justify-between gap-3">
            <p role="alert" className="text-sm text-error">
              {t("dictation.cloudGateway.loadFailed")}
            </p>
            <Button variant="secondary" onClick={load} disabled={busy}>
              {t("dictation.retry")}
            </Button>
          </div>
        ) : !view ? (
          <p className="text-sm text-text/60">{t("common.loading")}</p>
        ) : (
          <>
            <div
              className="grid grid-cols-2 gap-2"
              role="group"
              aria-label={t("dictation.cloudGateway.modeLabel")}
            >
              {(["cloud_first", "local_only"] as const).map(
                (mode: RecognitionMode) => (
                  <Button
                    key={mode}
                    variant={
                      view.recognition_mode === mode
                        ? "primary-soft"
                        : "secondary"
                    }
                    aria-pressed={view.recognition_mode === mode}
                    disabled={busy}
                    onClick={() => {
                      setView({ ...view, recognition_mode: mode });
                      setDirty(true);
                    }}
                  >
                    {t(`dictation.cloudGateway.modes.${mode}`)}
                  </Button>
                ),
              )}
            </div>
            {bannerKey && (
              <p className="text-xs text-text/65 flex items-center gap-1.5">
                <ShieldCheck
                  size={14}
                  className="shrink-0 text-logo-primary"
                  aria-hidden="true"
                />
                <span>{t(`dictation.cloudGateway.banners.${bannerKey}`)}</span>
              </p>
            )}

            <div className="grid gap-3 pt-1">
              <label className="grid gap-1.5 text-sm">
                {t("dictation.cloudGateway.urlLabel")}
                <Input
                  value={view.gateway_url}
                  disabled={busy}
                  spellCheck={false}
                  placeholder={t("dictation.cloudGateway.urlPlaceholder")}
                  onChange={(event) => {
                    setView({ ...view, gateway_url: event.target.value });
                    setDirty(true);
                  }}
                />
              </label>

              <div className="space-y-1.5">
                <p className="text-sm">
                  {t(
                    view.token_configured
                      ? isMac
                        ? "dictation.cloudGateway.tokenConfiguredMac"
                        : "dictation.cloudGateway.tokenConfigured"
                      : "dictation.cloudGateway.tokenMissing",
                  )}
                </p>
                <div className="flex flex-wrap items-center gap-2">
                  <Input
                    type="password"
                    autoComplete="new-password"
                    spellCheck={false}
                    value={tokenInput}
                    disabled={busy}
                    aria-label={t("dictation.cloudGateway.tokenLabel")}
                    placeholder={t(
                      isMac
                        ? "dictation.cloudGateway.tokenPlaceholderMac"
                        : "dictation.cloudGateway.tokenPlaceholder",
                    )}
                    onChange={(event) => setTokenInput(event.target.value)}
                    className="min-w-48 flex-1"
                  />
                  <Button
                    variant="secondary"
                    disabled={busy || tokenInput.trim().length < 12}
                    onClick={() => void updateToken(false)}
                  >
                    {t("dictation.cloudGateway.saveToken")}
                  </Button>
                  {view.token_configured && (
                    <Button
                      variant="danger-ghost"
                      disabled={busy}
                      onClick={() => void updateToken(true)}
                    >
                      {t("dictation.cloudGateway.removeToken")}
                    </Button>
                  )}
                </div>
              </div>

              <label className="flex items-start gap-2 text-sm cursor-pointer pt-1">
                <input
                  type="checkbox"
                  className="mt-1 accent-logo-primary"
                  checked={view.auto_sync_vocabulary}
                  disabled={busy}
                  onChange={(event) => {
                    setView({
                      ...view,
                      auto_sync_vocabulary: event.target.checked,
                    });
                    setDirty(true);
                  }}
                />
                <span>
                  {t("dictation.cloudGateway.autoSync")}
                  <span className="block text-xs text-text/60 mt-0.5">
                    {t("dictation.cloudGateway.autoSyncHint")}
                  </span>
                </span>
              </label>
            </div>

            {testResult && (
              <div
                role="status"
                className="rounded-md border border-mid-gray/20 bg-mid-gray/5 px-3 py-2 text-xs space-y-1"
              >
                <p className="font-medium">
                  {testResult.authenticated
                    ? t("dictation.cloudGateway.testStatusAuthenticated", {
                        device: testResult.device_id ?? "-",
                        ms: testResult.latency_ms,
                      })
                    : t("dictation.cloudGateway.testStatusUnauthorized", {
                        ms: testResult.latency_ms,
                      })}
                </p>
                {testResult.authenticated && (
                  <p className="text-text/65">
                    {t("dictation.cloudGateway.testStatusUpstream", {
                      asr: testResult.asr_ready
                        ? (testResult.asr_model ?? "OK")
                        : t("dictation.cloudGateway.notReady"),
                      cleanup: testResult.cleanup_ready
                        ? (testResult.cleanup_model ?? "OK")
                        : t("dictation.cloudGateway.notReady"),
                      vocab: testResult.vocabulary_count,
                      rev: testResult.vocabulary_revision,
                    })}
                  </p>
                )}
              </div>
            )}

            <div className="flex flex-wrap items-center justify-between gap-2 border-t border-mid-gray/15 pt-3">
              <div className="text-xs text-text/60">
                {t("dictation.cloudGateway.syncSummary", {
                  count: view.synced_active_count,
                  revision: view.last_synced_revision,
                })}
              </div>
              <div className="flex flex-wrap items-center gap-2">
                <Button
                  variant="secondary"
                  size="sm"
                  disabled={
                    busy ||
                    testing ||
                    !view.token_configured ||
                    !normalizeGatewayUrlInput(view.gateway_url)
                  }
                  onClick={() => void runTest()}
                >
                  {testing
                    ? t("dictation.cloudGateway.testing")
                    : t("dictation.cloudGateway.testConnection")}
                </Button>
                <Button
                  variant="secondary"
                  size="sm"
                  disabled={
                    busy ||
                    syncing ||
                    !view.token_configured ||
                    !normalizeGatewayUrlInput(view.gateway_url)
                  }
                  onClick={() => void runSync(false)}
                >
                  <RefreshCw
                    size={13}
                    className={`mr-1 ${syncing ? "animate-spin" : ""}`}
                    aria-hidden="true"
                  />
                  {t("dictation.cloudGateway.syncNow")}
                </Button>
                <Button
                  disabled={busy || !dirty}
                  onClick={() => void saveConfig()}
                >
                  {t("dictation.cloudGateway.save")}
                </Button>
              </div>
            </div>
          </>
        )}
      </div>
    </SettingsGroup>
  );
};
