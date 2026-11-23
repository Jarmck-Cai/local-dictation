import { invoke } from "@tauri-apps/api/core";

export const CLOUD_GATEWAY_REFRESH = "cloud-gateway-refresh";

export type RecognitionMode = "cloud_first" | "local_only";

export interface CloudGatewayConfig {
  recognition_mode: RecognitionMode;
  gateway_url: string;
  auto_sync_vocabulary: boolean;
}

export interface CloudGatewayView extends CloudGatewayConfig {
  token_configured: boolean;
  gateway_ready: boolean;
  effective_mode: RecognitionMode;
  last_synced_revision: number;
  last_synced_at_ms: number | null;
  synced_active_count: number;
}

export interface GatewayConnectionTestResult {
  reachable: boolean;
  authenticated: boolean;
  device_id: string | null;
  asr_ready: boolean;
  cleanup_ready: boolean;
  asr_model: string | null;
  cleanup_model: string | null;
  vocabulary_revision: number;
  vocabulary_count: number;
  latency_ms: number;
}

export interface GatewayVocabularySyncResult {
  uploaded_candidates: number;
  upserted_remote: number;
  imported_local: number;
  revision: number;
  total_active: number;
}

export function normalizeGatewayUrlInput(raw: string): string {
  return raw.trim().replace(/\/+$/, "");
}

export function isValidGatewayUrlInput(raw: string): boolean {
  const cleaned = normalizeGatewayUrlInput(raw);
  if (!cleaned) return true;
  if (cleaned.length > 512) return false;
  if (/[\s@#?\\]/.test(cleaned)) return false;
  try {
    const parsed = new URL(cleaned);
    if (parsed.protocol !== "https:" && parsed.protocol !== "http:") {
      return false;
    }
    return Boolean(parsed.hostname) && !parsed.username && !parsed.password;
  } catch {
    return false;
  }
}

export function gatewayEffectiveBannerKey(
  view: Pick<
    CloudGatewayView,
    "recognition_mode" | "gateway_ready" | "effective_mode"
  >,
): "cloudActive" | "cloudUnconfiguredLocal" | "localOnly" {
  if (view.recognition_mode === "local_only") {
    return "localOnly";
  }
  if (view.gateway_ready && view.effective_mode === "cloud_first") {
    return "cloudActive";
  }
  return "cloudUnconfiguredLocal";
}

/** A saved cloud route remains usable, while a new route needs a passing test. */
export function canEnableMacGateway(
  draft: Pick<CloudGatewayView, "gateway_url" | "token_configured"> | null,
  savedCloudUrl: string,
  verifiedUrl: string,
): boolean {
  if (!draft?.token_configured) return false;
  const url = normalizeGatewayUrlInput(draft.gateway_url);
  return url !== "" && (url === savedCloudUrl || url === verifiedUrl);
}

export function classifyGatewayErrorCode(
  err: unknown,
):
  | "invalid_gateway_url"
  | "invalid_gateway_token"
  | "missing_gateway_url"
  | "missing_gateway_token"
  | "gateway_unauthorized"
  | "gateway_timeout"
  | "gateway_unreachable"
  | "generic" {
  const code = typeof err === "string" ? err.trim() : "";
  if (
    code === "invalid_gateway_url" ||
    code === "invalid_gateway_token" ||
    code === "missing_gateway_url" ||
    code === "missing_gateway_token" ||
    code === "gateway_unauthorized" ||
    code === "gateway_timeout" ||
    code === "gateway_unreachable"
  ) {
    return code;
  }
  return "generic";
}

const notifyGatewayRefresh = () => {
  if (typeof window !== "undefined") {
    window.dispatchEvent(new Event(CLOUD_GATEWAY_REFRESH));
  }
};

export const cloudGatewayCommands = {
  getConfig: () => invoke<CloudGatewayView>("get_cloud_gateway_config"),
  saveConfig: async (config: CloudGatewayConfig) => {
    const view = await invoke<CloudGatewayView>("save_cloud_gateway_config", {
      config,
    });
    notifyGatewayRefresh();
    return view;
  },
  setToken: async (token: string) => {
    const view = await invoke<CloudGatewayView>("set_cloud_gateway_token", {
      token,
    });
    notifyGatewayRefresh();
    return view;
  },
  clearToken: async () => {
    const view = await invoke<CloudGatewayView>("clear_cloud_gateway_token");
    notifyGatewayRefresh();
    return view;
  },
  testConnection: (gatewayUrl?: string) =>
    invoke<GatewayConnectionTestResult>("test_cloud_gateway_connection", {
      gatewayUrl: gatewayUrl ?? null,
    }),
  syncVocabulary: async (fullSync = false) => {
    const res = await invoke<GatewayVocabularySyncResult>(
      "sync_cloud_gateway_vocabulary",
      { fullSync },
    );
    notifyGatewayRefresh();
    return res;
  },
};
