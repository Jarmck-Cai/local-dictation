import { useCallback, useEffect, useState, type ReactNode } from "react";
import { listen } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import { ChevronDown, RefreshCw } from "lucide-react";
import { toast } from "sonner";
import {
  cloudGatewayCommands,
  classifyGatewayErrorCode,
  gatewayEffectiveBannerKey,
  isValidGatewayUrlInput,
  normalizeGatewayUrlInput,
  type CloudGatewayConfig,
  type CloudGatewayView,
  type GatewayConnectionTestResult,
  type RecognitionMode,
} from "@/lib/cloudGateway";
import { Button } from "../ui/Button";
import { Input } from "../ui/Input";
import { SettingsGroup } from "../ui/SettingsGroup";

type ConnectionDraft = Partial<
  Pick<CloudGatewayConfig, "gateway_url" | "auto_sync_vocabulary">
>;

export const useCloudGatewaySettings = () => {
  const { t } = useTranslation();
  const [view, setView] = useState<CloudGatewayView | null>(null);
  const [busy, setBusy] = useState(false);
  const [testing, setTesting] = useState(false);
  const [syncing, setSyncing] = useState(false);
  const [loadFailed, setLoadFailed] = useState(false);
  const [tokenInput, setTokenInput] = useState("");
  const [modeDraft, setModeDraft] = useState<RecognitionMode | null>(null);
  const [connectionDraft, setConnectionDraft] = useState<ConnectionDraft>({});
  const [testResult, setTestResult] =
    useState<GatewayConnectionTestResult | null>(null);

  const mode = modeDraft ?? view?.recognition_mode;
  const url = connectionDraft.gateway_url ?? view?.gateway_url ?? "";
  const autoSync =
    connectionDraft.auto_sync_vocabulary ?? view?.auto_sync_vocabulary ?? false;
  const modeDirty = mode !== view?.recognition_mode;
  const connectionDirty =
    view !== null &&
    (url !== view.gateway_url || autoSync !== view.auto_sync_vocabulary);

  const updateConnectionDraft = (change: ConnectionDraft) => {
    setConnectionDraft((previous) => ({ ...previous, ...change }));
    if (change.gateway_url !== undefined) setTestResult(null);
  };

  const load = useCallback(async () => {
    setBusy(true);
    setLoadFailed(false);
    try {
      setView(await cloudGatewayCommands.getConfig());
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
      if (active) setView(event.payload);
    })
      .then((fn) => {
        if (!active) fn();
        else unlisten = fn;
      })
      .catch(() => {
        // Configuration can still be read and saved through commands.
      });
    return () => {
      active = false;
      unlisten?.();
    };
  }, [load]);

  const saveConfig = async (
    config: CloudGatewayConfig,
    clearDraft: () => void,
  ) => {
    if (!isValidGatewayUrlInput(config.gateway_url)) {
      toast.error(t("dictation.cloudGateway.errors.invalid_gateway_url"));
      return;
    }
    setBusy(true);
    try {
      const updated = await cloudGatewayCommands.saveConfig({
        ...config,
        gateway_url: normalizeGatewayUrlInput(config.gateway_url),
      });
      setView(updated);
      clearDraft();
      toast.success(t("dictation.cloudGateway.saved"));
    } catch (err) {
      const code = classifyGatewayErrorCode(err);
      toast.error(t(`dictation.cloudGateway.errors.${code}`));
    } finally {
      setBusy(false);
    }
  };

  const saveMode = () => {
    if (!view || !mode) return;
    void saveConfig(
      {
        recognition_mode: mode,
        gateway_url: view.gateway_url,
        auto_sync_vocabulary: view.auto_sync_vocabulary,
      },
      () => setModeDraft(null),
    );
  };

  const saveConnection = () => {
    if (!view) return;
    void saveConfig(
      {
        recognition_mode: view.recognition_mode,
        gateway_url: url,
        auto_sync_vocabulary: autoSync,
      },
      () => setConnectionDraft({}),
    );
  };

  const updateToken = async (clear: boolean) => {
    setBusy(true);
    try {
      const updated = clear
        ? await cloudGatewayCommands.clearToken()
        : await cloudGatewayCommands.setToken(tokenInput.trim());
      setView(updated);
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
    const cleanedUrl = normalizeGatewayUrlInput(url);
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
      if (connectionDirty && view) {
        const saved = await cloudGatewayCommands.saveConfig({
          recognition_mode: view.recognition_mode,
          gateway_url: normalizeGatewayUrlInput(url),
          auto_sync_vocabulary: autoSync,
        });
        setView(saved);
        setConnectionDraft({});
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

  return {
    view,
    busy: busy || testing || syncing,
    testing,
    syncing,
    loadFailed,
    mode,
    setModeDraft,
    modeDirty,
    url,
    autoSync,
    connectionDirty,
    updateConnectionDraft,
    tokenInput,
    setTokenInput,
    testResult,
    load,
    saveMode,
    saveConnection,
    updateToken,
    runTest,
    runSync,
  };
};

interface GatewaySettingsProps {
  settings: ReturnType<typeof useCloudGatewaySettings>;
}

export const RecognitionModeSettings = ({ settings }: GatewaySettingsProps) => {
  const { t } = useTranslation();
  const {
    view,
    busy,
    loadFailed,
    mode,
    modeDirty,
    setModeDraft,
    load,
    saveMode,
  } = settings;

  const bannerKey = view ? gatewayEffectiveBannerKey(view) : null;

  return (
    <SettingsGroup title={t("dictation.cloudGateway.modeLabel")}>
      <div className="space-y-2 px-4 py-3">
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
            <div className="flex flex-wrap items-center justify-between gap-2">
              <div
                className="flex flex-wrap gap-2"
                role="group"
                aria-label={t("dictation.cloudGateway.modeLabel")}
              >
                {(["cloud_first", "local_only"] as const).map((option) => (
                  <Button
                    key={option}
                    variant={mode === option ? "primary-soft" : "secondary"}
                    aria-pressed={mode === option}
                    disabled={busy}
                    onClick={() =>
                      setModeDraft(
                        option === view.recognition_mode ? null : option,
                      )
                    }
                  >
                    {t(`dictation.cloudGateway.modes.${option}`)}
                  </Button>
                ))}
              </div>
              <Button disabled={busy || !modeDirty} onClick={saveMode}>
                {t("dictation.cloudGateway.saveRecognitionMode")}
              </Button>
            </div>
            {bannerKey && (
              <p className="text-xs text-text/65" role="status">
                {t(`dictation.cloudGateway.banners.${bannerKey}`)}
              </p>
            )}
          </>
        )}
      </div>
    </SettingsGroup>
  );
};

export const CloudGatewaySettings = ({
  settings,
  children,
}: GatewaySettingsProps & { children?: ReactNode }) => {
  const { t } = useTranslation();
  const {
    view,
    busy,
    testing,
    syncing,
    loadFailed,
    url,
    autoSync,
    connectionDirty,
    updateConnectionDraft,
    tokenInput,
    setTokenInput,
    testResult,
    saveConnection,
    updateToken,
    runTest,
    runSync,
  } = settings;

  return (
    <details className="group rounded-lg border border-mid-gray/20 bg-background">
      <summary className="flex cursor-pointer list-none items-center justify-between gap-3 rounded-lg px-4 py-3 hover:bg-mid-gray/5 focus-visible:outline-2 focus-visible:outline-logo-primary [&::-webkit-details-marker]:hidden">
        <span className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
          <span className="text-sm font-medium">
            {t("dictation.cloudGateway.title")}
          </span>
          <span className="text-xs text-text/65">
            {t(
              loadFailed
                ? "dictation.cloudGateway.loadFailed"
                : !view
                  ? "common.loading"
                  : view.gateway_ready
                    ? "dictation.cloudGateway.connectionConfigured"
                    : "dictation.cloudGateway.connectionUnconfigured",
            )}
          </span>
        </span>
        <ChevronDown
          size={16}
          className="shrink-0 text-text/65 group-open:rotate-180"
          aria-hidden="true"
        />
      </summary>
      <div className="space-y-4 border-t border-mid-gray/15 p-4">
        <p className="text-xs text-text/65">
          {t("dictation.cloudGateway.description")}
        </p>
        {view && !loadFailed && (
          <>
            <div className="grid gap-3 pt-1">
              <label className="grid gap-1.5 text-sm">
                {t("dictation.cloudGateway.urlLabel")}
                <Input
                  value={url}
                  disabled={busy}
                  spellCheck={false}
                  placeholder={t("dictation.cloudGateway.urlPlaceholder")}
                  onChange={(event) =>
                    updateConnectionDraft({ gateway_url: event.target.value })
                  }
                />
              </label>

              <div className="space-y-1.5">
                <p className="text-sm">
                  {t(
                    view.token_configured
                      ? "dictation.cloudGateway.tokenConfigured"
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
                    placeholder={t("dictation.cloudGateway.tokenPlaceholder")}
                    onChange={(event) => setTokenInput(event.target.value)}
                    className="min-w-0 basis-48 flex-1"
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
                  checked={autoSync}
                  disabled={busy}
                  onChange={(event) =>
                    updateConnectionDraft({
                      auto_sync_vocabulary: event.target.checked,
                    })
                  }
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
                    !view.token_configured ||
                    !normalizeGatewayUrlInput(url)
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
                  className="inline-flex items-center gap-1"
                  disabled={
                    busy ||
                    !view.token_configured ||
                    !normalizeGatewayUrlInput(url)
                  }
                  onClick={() => void runSync(false)}
                >
                  <RefreshCw
                    size={13}
                    className={syncing ? "animate-spin" : undefined}
                    aria-hidden="true"
                  />
                  {t("dictation.cloudGateway.syncNow")}
                </Button>
                <Button
                  disabled={busy || !connectionDirty}
                  onClick={saveConnection}
                >
                  {t("dictation.cloudGateway.save")}
                </Button>
              </div>
            </div>
          </>
        )}
        {children}
      </div>
    </details>
  );
};
