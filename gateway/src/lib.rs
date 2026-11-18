//! Lightweight stateless-audio dictation and SQLite vocabulary sync gateway.

pub mod auth;
pub mod policy;
pub mod upstream;
pub mod vocab;

use crate::auth::{SecretString, TokenRegistry};
use crate::upstream::{AudioPayload, CleanupStrength};
use crate::vocab::{VocabularyStore, VocabularySyncRequest, VocabularySyncResponse};
use axum::extract::{DefaultBodyLimit, Multipart, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
pub const DEFAULT_ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com";
pub const DEFAULT_OPENAI_ASR_MODEL: &str = "gpt-transcribe";
pub const DEFAULT_ANTHROPIC_CLEANUP_MODEL: &str = "claude-haiku-5-5";
pub const DEFAULT_MAX_AUDIO_BYTES: usize = 16 * 1024 * 1024;
pub const DEFAULT_ASR_TIMEOUT_MS: u64 = 20_000;
pub const DEFAULT_CLEANUP_TIMEOUT_MS: u64 = 8_000;

pub struct GatewayConfig {
    pub bind_addr: String,
    pub db_path: PathBuf,
    pub tokens: TokenRegistry,
    pub openai_api_key: Option<SecretString>,
    pub openai_base_url: String,
    pub openai_asr_model: String,
    pub anthropic_api_key: Option<SecretString>,
    pub anthropic_base_url: String,
    pub anthropic_cleanup_model: String,
    pub max_audio_bytes: usize,
    pub asr_timeout_ms: u64,
    pub cleanup_timeout_ms: u64,
}

impl GatewayConfig {
    pub fn from_env() -> Result<Self, String> {
        let bind_addr =
            std::env::var("GATEWAY_BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:8787".to_string());
        let db_path = PathBuf::from(
            std::env::var("GATEWAY_DB_PATH").unwrap_or_else(|_| "./data/gateway.db".to_string()),
        );
        let raw_tokens = std::env::var("GATEWAY_AUTH_TOKENS")
            .map_err(|_| "GATEWAY_AUTH_TOKENS environment variable is required".to_string())?;
        let tokens = TokenRegistry::parse(&raw_tokens)
            .map_err(|code| format!("invalid GATEWAY_AUTH_TOKENS: {code}"))?;

        let openai_api_key = std::env::var("OPENAI_API_KEY")
            .ok()
            .and_then(SecretString::new);
        let openai_base_url = std::env::var("OPENAI_BASE_URL")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_OPENAI_BASE_URL.to_string());
        let openai_asr_model = std::env::var("OPENAI_ASR_MODEL")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_OPENAI_ASR_MODEL.to_string());

        let anthropic_api_key = std::env::var("ANTHROPIC_API_KEY")
            .ok()
            .and_then(SecretString::new);
        let anthropic_base_url = std::env::var("ANTHROPIC_BASE_URL")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_ANTHROPIC_BASE_URL.to_string());
        let anthropic_cleanup_model = std::env::var("ANTHROPIC_CLEANUP_MODEL")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_ANTHROPIC_CLEANUP_MODEL.to_string());

        let max_audio_bytes = std::env::var("GATEWAY_MAX_AUDIO_BYTES")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .map(|v| v.clamp(64 * 1024, 25 * 1024 * 1024))
            .unwrap_or(DEFAULT_MAX_AUDIO_BYTES);
        let asr_timeout_ms = std::env::var("GATEWAY_ASR_TIMEOUT_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .map(|v| v.clamp(2_000, 120_000))
            .unwrap_or(DEFAULT_ASR_TIMEOUT_MS);
        let cleanup_timeout_ms = std::env::var("GATEWAY_CLEANUP_TIMEOUT_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .map(|v| v.clamp(1_000, 30_000))
            .unwrap_or(DEFAULT_CLEANUP_TIMEOUT_MS);

        Ok(Self {
            bind_addr,
            db_path,
            tokens,
            openai_api_key,
            openai_base_url,
            openai_asr_model,
            anthropic_api_key,
            anthropic_base_url,
            anthropic_cleanup_model,
            max_audio_bytes,
            asr_timeout_ms,
            cleanup_timeout_ms,
        })
    }
}

pub struct AppState {
    pub config: GatewayConfig,
    pub vocab: VocabularyStore,
    pub http: reqwest::Client,
}

impl AppState {
    pub fn new(config: GatewayConfig, vocab: VocabularyStore) -> Result<Arc<Self>, String> {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(4))
            .pool_idle_timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| format!("failed to build HTTP client: {e}"))?;
        Ok(Arc::new(Self {
            config,
            vocab,
            http,
        }))
    }
}

pub fn build_router(state: Arc<AppState>) -> Router {
    // Allow 512 KiB multipart envelope headroom above max_audio_bytes.
    let body_limit = state.config.max_audio_bytes.saturating_add(512 * 1024);
    Router::new()
        .route("/healthz", get(handle_healthz))
        .route("/v1/status", get(handle_status))
        .route("/v1/vocabulary", get(handle_get_vocabulary))
        .route("/v1/vocabulary/sync", post(handle_sync_vocabulary))
        .route("/v1/dictate", post(handle_dictate))
        .layer(DefaultBodyLimit::max(body_limit))
        .with_state(state)
}

#[derive(Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub service: String,
    pub version: String,
}

#[derive(Serialize, Deserialize)]
pub struct StatusResponse {
    pub status: String,
    pub version: String,
    pub device_id: String,
    pub asr_ready: bool,
    pub cleanup_ready: bool,
    pub asr_model: String,
    pub cleanup_model: String,
    pub vocabulary_revision: i64,
    pub vocabulary_count: u32,
}

#[derive(Serialize, Deserialize)]
pub struct ErrorBody {
    pub error: String,
}

struct ApiError {
    status: StatusCode,
    code: &'static str,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorBody {
                error: self.code.to_string(),
            }),
        )
            .into_response()
    }
}

fn authenticate<'a>(state: &'a AppState, headers: &HeaderMap) -> Result<&'a str, ApiError> {
    let header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or(ApiError {
            status: StatusCode::UNAUTHORIZED,
            code: "missing_authorization",
        })?;
    state
        .config
        .tokens
        .verify_authorization_header(header)
        .ok_or(ApiError {
            status: StatusCode::UNAUTHORIZED,
            code: "invalid_device_token",
        })
}

async fn handle_healthz() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".into(),
        service: "local-dictation-gateway".into(),
        version: env!("CARGO_PKG_VERSION").into(),
    })
}

async fn handle_status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<StatusResponse>, ApiError> {
    let device_id = authenticate(&state, &headers)?;
    let (vocabulary_revision, vocabulary_count) =
        state.vocab.summary().map_err(|code| ApiError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code,
        })?;
    Ok(Json(StatusResponse {
        status: "ok".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        device_id: device_id.to_string(),
        asr_ready: state.config.openai_api_key.is_some(),
        cleanup_ready: state.config.anthropic_api_key.is_some(),
        asr_model: state.config.openai_asr_model.clone(),
        cleanup_model: state.config.anthropic_cleanup_model.clone(),
        vocabulary_revision,
        vocabulary_count,
    }))
}

#[derive(Default, Deserialize)]
struct VocabularyQuery {
    #[serde(default)]
    since_revision: i64,
}

async fn handle_get_vocabulary(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<VocabularyQuery>,
) -> Result<Json<VocabularySyncResponse>, ApiError> {
    let _device_id = authenticate(&state, &headers)?;
    let response = state
        .vocab
        .list_since(query.since_revision)
        .map_err(|code| ApiError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code,
        })?;
    Ok(Json(response))
}

async fn handle_sync_vocabulary(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<VocabularySyncRequest>,
) -> Result<Json<VocabularySyncResponse>, ApiError> {
    let device_id = authenticate(&state, &headers)?;
    let response = state.vocab.sync(device_id, payload).map_err(|code| {
        let status = match code {
            "vocabulary_sync_batch_too_large" | "vocabulary_capacity_exceeded" => {
                StatusCode::PAYLOAD_TOO_LARGE
            }
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        ApiError { status, code }
    })?;
    log::info!(
        "vocabulary_sync device={} upserted={} revision={} total_active={}",
        device_id,
        response.upserted,
        response.revision,
        response.total_active
    );
    Ok(Json(response))
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DictateMetadata {
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default = "default_true")]
    pub cleanup_enabled: bool,
    #[serde(default)]
    pub strength: CleanupStrength,
    #[serde(default = "default_true")]
    pub natural_paragraphs: bool,
    #[serde(default)]
    pub hint_terms: Vec<String>,
}

fn default_true() -> bool {
    true
}

impl Default for DictateMetadata {
    fn default() -> Self {
        Self {
            language: None,
            cleanup_enabled: true,
            strength: CleanupStrength::Light,
            natural_paragraphs: true,
            hint_terms: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DictateResponse {
    pub raw_text: String,
    pub formatted_text: String,
    pub cleanup_applied: bool,
    pub fallback_reason: Option<String>,
    pub asr_ms: u64,
    pub cleanup_ms: u64,
    pub vocabulary_revision: i64,
}

async fn handle_dictate(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<Json<DictateResponse>, ApiError> {
    let device_id = authenticate(&state, &headers)?.to_string();
    let openai_key = state.config.openai_api_key.as_ref().ok_or(ApiError {
        status: StatusCode::SERVICE_UNAVAILABLE,
        code: "asr_not_configured",
    })?;

    let mut audio_bytes: Option<Vec<u8>> = None;
    let mut upload_name: Option<String> = None;
    let mut upload_content_type: Option<String> = None;
    let mut meta = DictateMetadata::default();

    while let Some(field) = multipart.next_field().await.map_err(|_| ApiError {
        status: StatusCode::BAD_REQUEST,
        code: "invalid_multipart_payload",
    })? {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "audio" | "file" => {
                upload_name = field.file_name().map(str::to_string);
                upload_content_type = field.content_type().map(str::to_string);
                let data = field.bytes().await.map_err(|_| ApiError {
                    status: StatusCode::PAYLOAD_TOO_LARGE,
                    code: "audio_too_large",
                })?;
                if data.len() > state.config.max_audio_bytes {
                    return Err(ApiError {
                        status: StatusCode::PAYLOAD_TOO_LARGE,
                        code: "audio_too_large",
                    });
                }
                audio_bytes = Some(data.to_vec());
            }
            "metadata" => {
                let text = field.text().await.map_err(|_| ApiError {
                    status: StatusCode::BAD_REQUEST,
                    code: "invalid_metadata_field",
                })?;
                if text.len() > 65_536 {
                    return Err(ApiError {
                        status: StatusCode::BAD_REQUEST,
                        code: "metadata_too_large",
                    });
                }
                meta = serde_json::from_str(&text).map_err(|_| ApiError {
                    status: StatusCode::BAD_REQUEST,
                    code: "invalid_metadata_json",
                })?;
            }
            "language" => {
                if let Ok(val) = field.text().await {
                    meta.language = Some(val);
                }
            }
            "cleanup_enabled" => {
                if let Ok(val) = field.text().await {
                    meta.cleanup_enabled = matches!(
                        val.trim().to_ascii_lowercase().as_str(),
                        "1" | "true" | "yes" | "on"
                    );
                }
            }
            "strength" => {
                if let Ok(val) = field.text().await {
                    meta.strength = match val.trim().to_ascii_lowercase().as_str() {
                        "faithful" => CleanupStrength::Faithful,
                        "clear" => CleanupStrength::Clear,
                        _ => CleanupStrength::Light,
                    };
                }
            }
            "natural_paragraphs" => {
                if let Ok(val) = field.text().await {
                    meta.natural_paragraphs = !matches!(
                        val.trim().to_ascii_lowercase().as_str(),
                        "0" | "false" | "no" | "off"
                    );
                }
            }
            "hint_terms" => {
                if let Ok(val) = field.text().await {
                    if let Ok(parsed) = serde_json::from_str::<Vec<String>>(&val) {
                        meta.hint_terms.extend(parsed);
                    } else {
                        meta.hint_terms.extend(
                            val.split(',')
                                .map(str::trim)
                                .filter(|s| !s.is_empty())
                                .map(str::to_string),
                        );
                    }
                }
            }
            _ => {}
        }
    }

    let raw_audio = audio_bytes.ok_or(ApiError {
        status: StatusCode::BAD_REQUEST,
        code: "missing_audio_field",
    })?;
    let audio_len = raw_audio.len();
    let audio_payload = AudioPayload::validate(
        raw_audio,
        upload_name.as_deref(),
        upload_content_type.as_deref(),
        state.config.max_audio_bytes,
    )
    .map_err(|code| ApiError {
        status: if code == "audio_too_large" {
            StatusCode::PAYLOAD_TOO_LARGE
        } else {
            StatusCode::BAD_REQUEST
        },
        code,
    })?;

    let prompt_terms = state.vocab.select_prompt_terms(&meta.hint_terms, 64);
    let vocabulary_revision = state.vocab.summary().map(|(rev, _)| rev).unwrap_or(0);

    // Step 1: Pure in-memory OpenAI ASR (`audio_payload` is consumed and dropped right here).
    let asr_started = Instant::now();
    let raw_text = upstream::transcribe_openai(
        &state.http,
        upstream::OpenAiAsrRequest {
            base_url: &state.config.openai_base_url,
            api_key: openai_key,
            model: &state.config.openai_asr_model,
            audio: audio_payload,
            language: meta.language.as_deref(),
            prompt_terms: &prompt_terms,
            timeout_ms: state.config.asr_timeout_ms,
        },
    )
    .await
    .map_err(|code| {
        log::warn!(
            "dictate_asr_failed device={} audio_bytes={} reason={}",
            device_id,
            audio_len,
            code
        );
        ApiError {
            status: StatusCode::BAD_GATEWAY,
            code,
        }
    })?;
    let asr_ms = asr_started.elapsed().as_millis().min(u64::MAX as u128) as u64;

    if raw_text.trim().is_empty() {
        log::info!(
            "dictate_empty device={} audio_bytes={} asr_ms={}",
            device_id,
            audio_len,
            asr_ms
        );
        return Ok(Json(DictateResponse {
            raw_text: String::new(),
            formatted_text: String::new(),
            cleanup_applied: false,
            fallback_reason: None,
            asr_ms,
            cleanup_ms: 0,
            vocabulary_revision,
        }));
    }

    if !meta.cleanup_enabled {
        let normalized = policy::normalize_output(&raw_text);
        log::info!(
            "dictate_ok device={} audio_bytes={} asr_ms={} cleanup_ms=0 cleanup=off",
            device_id,
            audio_len,
            asr_ms
        );
        return Ok(Json(DictateResponse {
            raw_text,
            formatted_text: normalized,
            cleanup_applied: false,
            fallback_reason: None,
            asr_ms,
            cleanup_ms: 0,
            vocabulary_revision,
        }));
    }

    let Some(anthropic_key) = state.config.anthropic_api_key.as_ref() else {
        let normalized = policy::normalize_output(&raw_text);
        return Ok(Json(DictateResponse {
            raw_text,
            formatted_text: normalized,
            cleanup_applied: false,
            fallback_reason: Some("cleanup_not_configured".to_string()),
            asr_ms,
            cleanup_ms: 0,
            vocabulary_revision,
        }));
    };

    // Step 2: Anthropic Claude formatting & spoken list structuring (graceful fallback on failure).
    let cleanup_started = Instant::now();
    let cleanup_result = upstream::cleanup_anthropic(
        &state.http,
        upstream::AnthropicCleanupRequest {
            base_url: &state.config.anthropic_base_url,
            api_key: anthropic_key,
            model: &state.config.anthropic_cleanup_model,
            raw: &raw_text,
            strength: meta.strength,
            natural_paragraphs: meta.natural_paragraphs,
            terms: &prompt_terms,
            max_budget_ms: state.config.cleanup_timeout_ms,
        },
    )
    .await;
    let cleanup_ms = cleanup_started.elapsed().as_millis().min(u64::MAX as u128) as u64;

    match cleanup_result {
        Ok(formatted_text) => {
            log::info!(
                "dictate_ok device={} audio_bytes={} asr_ms={} cleanup_ms={} cleanup=applied",
                device_id,
                audio_len,
                asr_ms,
                cleanup_ms
            );
            Ok(Json(DictateResponse {
                raw_text,
                formatted_text,
                cleanup_applied: true,
                fallback_reason: None,
                asr_ms,
                cleanup_ms,
                vocabulary_revision,
            }))
        }
        Err(reason) => {
            log::warn!(
                "dictate_cleanup_fallback device={} audio_bytes={} asr_ms={} cleanup_ms={} reason={}",
                device_id,
                audio_len,
                asr_ms,
                cleanup_ms,
                reason
            );
            let fallback_text = policy::normalize_output(&raw_text);
            Ok(Json(DictateResponse {
                raw_text,
                formatted_text: fallback_text,
                cleanup_applied: false,
                fallback_reason: Some(reason.to_string()),
                asr_ms,
                cleanup_ms,
                vocabulary_revision,
            }))
        }
    }
}
