//! Self-hosted cloud dictation gateway client and cross-device vocabulary synchronization.
//!
//! - Client holds ZERO upstream API keys (`OPENAI_API_KEY` / `ANTHROPIC_API_KEY`).
//! - The gateway URL is saved in settings. The device token uses DPAPI on
//!   Windows and the login Keychain on macOS.
//! - Live dictation sends audio frames while recording. If streaming is
//!   unavailable, the same recording can use the existing in-memory WAV upload
//!   and then fall back to local recognition.

use crate::dictation_cleanup::{self, CleanupSession, CleanupStrength, SecretBytes};
#[cfg(not(target_os = "macos"))]
use crate::dictation_cleanup::{protect_secret_bytes, unprotect_secret_bytes};
use crate::managers::transcription::TranscriptionManager;
use crate::settings::{get_settings, write_settings, RecognitionMode};
use hound::{SampleFormat, WavSpec, WavWriter};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};

mod live;
pub(crate) use live::AudioRoute;

#[cfg(not(target_os = "macos"))]
const TOKEN_FILE: &str = "gateway-token.dpapi";
const SYNC_STATE_FILE: &str = "gateway-vocab-sync.json";
const MAX_STATE_BYTES: u64 = 512 * 1024;
const CONNECT_TIMEOUT_MS: u64 = 2_500;
const BASE_DICTATE_TIMEOUT_MS: u64 = 12_000;
const MAX_DICTATE_TIMEOUT_MS: u64 = 25_000;
const MAX_LIVE_FINALIZE_TIMEOUT: Duration = Duration::from_secs(10);

static GATEWAY_IO: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));
static MULTIPART_SEQ: AtomicU64 = AtomicU64::new(1);
static AUTO_SYNC_RUNNING: AtomicBool = AtomicBool::new(false);
static AUTO_SYNC_PENDING: AtomicBool = AtomicBool::new(false);

static GATEWAY_HTTP: Lazy<Result<reqwest::Client, &'static str>> = Lazy::new(|| {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .connect_timeout(Duration::from_millis(CONNECT_TIMEOUT_MS))
        .pool_idle_timeout(Duration::from_secs(60))
        .build()
        .map_err(|_| "gateway_transport_unavailable")
});

#[derive(Clone, Serialize, Deserialize, specta::Type)]
pub struct CloudGatewayConfig {
    pub recognition_mode: RecognitionMode,
    pub gateway_url: String,
    pub auto_sync_vocabulary: bool,
}

#[derive(Clone, Serialize, Deserialize, specta::Type)]
pub struct CloudGatewayView {
    pub recognition_mode: RecognitionMode,
    pub gateway_url: String,
    pub auto_sync_vocabulary: bool,
    pub token_configured: bool,
    pub gateway_ready: bool,
    pub effective_mode: RecognitionMode,
    pub last_synced_revision: i64,
    pub last_synced_at_ms: Option<i64>,
    pub synced_active_count: u32,
}

#[derive(Clone, Serialize, Deserialize, specta::Type)]
pub struct GatewayConnectionTestResult {
    pub reachable: bool,
    pub authenticated: bool,
    pub device_id: Option<String>,
    pub asr_ready: bool,
    pub cleanup_ready: bool,
    pub asr_model: Option<String>,
    pub cleanup_model: Option<String>,
    pub vocabulary_revision: i64,
    pub vocabulary_count: u32,
    pub latency_ms: u64,
}

#[derive(Clone, Serialize, Deserialize, specta::Type)]
pub struct GatewayVocabularySyncResult {
    pub uploaded_candidates: u32,
    pub upserted_remote: u32,
    pub imported_local: u32,
    pub revision: i64,
    pub total_active: u32,
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct PersistedSyncState {
    #[serde(default)]
    last_synced_revision: i64,
    #[serde(default)]
    last_synced_at_ms: Option<i64>,
    #[serde(default)]
    synced_active_count: u32,
    #[serde(default)]
    synced_terms: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct WireVocabularyEntry {
    term: String,
    kind: String,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    context: Option<String>,
    #[serde(default)]
    count: u32,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default)]
    updated_at_ms: i64,
    #[serde(default)]
    revision: i64,
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize)]
struct WireVocabularySyncResponse {
    revision: i64,
    total_active: u32,
    upserted: u32,
    entries: Vec<WireVocabularyEntry>,
}

#[derive(Deserialize)]
struct WireStatusResponse {
    device_id: String,
    asr_ready: bool,
    cleanup_ready: bool,
    asr_model: String,
    cleanup_model: String,
    vocabulary_revision: i64,
    vocabulary_count: u32,
}

#[derive(Deserialize)]
pub(crate) struct WireDictateResponse {
    pub raw_text: String,
    pub formatted_text: String,
    pub cleanup_applied: bool,
    pub fallback_reason: Option<String>,
    pub asr_ms: u64,
    pub cleanup_ms: u64,
}

/// Pure URL validation: accepts empty string (unconfigured) or valid `http(s)://host[:port][/prefix]`
/// without credentials, queries, fragments, backslashes, or control characters.
pub fn validate_gateway_url(raw: &str) -> Result<String, &'static str> {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    if trimmed.len() > 512 {
        return Err("invalid_gateway_url");
    }
    let (scheme, remainder) = trimmed.split_once("://").ok_or("invalid_gateway_url")?;
    if !matches!(scheme, "https" | "http")
        || remainder.is_empty()
        || remainder
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '@' | '?' | '#' | '\\'))
    {
        return Err("invalid_gateway_url");
    }
    let authority = remainder.split('/').next().ok_or("invalid_gateway_url")?;
    if authority.is_empty() {
        return Err("invalid_gateway_url");
    }
    let port_part = if let Some(rest) = authority.strip_prefix('[') {
        let (_, after_bracket) = rest.split_once(']').ok_or("invalid_gateway_url")?;
        after_bracket
    } else if let Some((host, _)) = authority.split_once(':') {
        if host.is_empty() {
            return Err("invalid_gateway_url");
        }
        &authority[host.len()..]
    } else {
        ""
    };
    if !port_part.is_empty() {
        let digits = port_part.strip_prefix(':').ok_or("invalid_gateway_url")?;
        if digits.is_empty()
            || !digits.chars().all(|c| c.is_ascii_digit())
            || digits.parse::<u16>().ok().filter(|p| *p > 0).is_none()
        {
            return Err("invalid_gateway_url");
        }
    }
    Ok(trimmed.to_string())
}

fn token_from_string(raw: String) -> Result<SecretBytes, &'static str> {
    let mut secret = SecretBytes(raw.into_bytes());
    while secret.0.last().is_some_and(u8::is_ascii_whitespace) {
        secret.0.pop();
    }
    let first = secret
        .0
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(secret.0.len());
    secret.0.drain(..first);
    if secret.0.len() < 12
        || secret.0.len() > 4_096
        || secret.0.iter().any(|b| !matches!(b, 0x21..=0x7e))
    {
        return Err("invalid_gateway_token");
    }
    Ok(secret)
}

#[cfg(not(target_os = "macos"))]
fn token_storage_path(app: &AppHandle) -> Result<PathBuf, &'static str> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|_| "gateway_storage_unavailable")?;
    std::fs::create_dir_all(&dir).map_err(|_| "gateway_storage_unavailable")?;
    Ok(dir.join(TOKEN_FILE))
}

fn sync_state_path(app: &AppHandle) -> Result<PathBuf, &'static str> {
    let dir = crate::portable::app_data_dir(app).map_err(|_| "gateway_storage_unavailable")?;
    std::fs::create_dir_all(&dir).map_err(|_| "gateway_storage_unavailable")?;
    Ok(dir.join(SYNC_STATE_FILE))
}

fn read_small_file(path: &Path, max_bytes: u64) -> Result<Vec<u8>, &'static str> {
    let meta = std::fs::metadata(path).map_err(|_| "gateway_storage_unavailable")?;
    if !meta.is_file() || meta.len() > max_bytes {
        return Err("gateway_storage_invalid");
    }
    std::fs::read(path).map_err(|_| "gateway_storage_unavailable")
}

fn load_token(app: &AppHandle) -> Result<SecretBytes, &'static str> {
    let decrypted = read_token(app)?;
    if decrypted.0.len() < 12 || decrypted.0.iter().any(|b| !matches!(b, 0x21..=0x7e)) {
        return Err("gateway_token_unavailable");
    }
    Ok(decrypted)
}

#[cfg(not(target_os = "macos"))]
fn read_token(app: &AppHandle) -> Result<SecretBytes, &'static str> {
    let path = token_storage_path(app)?;
    if !path.exists() {
        return Err("missing_gateway_token");
    }
    let encrypted = read_small_file(&path, 65_536)?;
    unprotect_secret_bytes(&encrypted).map_err(|_| "gateway_token_unavailable")
}

#[cfg(target_os = "macos")]
fn read_token(_app: &AppHandle) -> Result<SecretBytes, &'static str> {
    crate::keychain::load(crate::keychain::GATEWAY)
        .map_err(|_| "gateway_token_unavailable")?
        .ok_or("missing_gateway_token")
}

#[cfg(target_os = "macos")]
fn persist_token(_app: &AppHandle, token: SecretBytes) -> Result<(), &'static str> {
    crate::keychain::save(crate::keychain::GATEWAY, &token)
        .map_err(|_| "gateway_token_unavailable")
}

#[cfg(not(target_os = "macos"))]
fn persist_token(app: &AppHandle, token: SecretBytes) -> Result<(), &'static str> {
    let encrypted = protect_secret_bytes(&token.0).map_err(|_| "gateway_token_unavailable")?;
    let path = token_storage_path(app)?;
    let pending = path.with_extension("dpapi.pending");
    std::fs::write(&pending, encrypted).map_err(|_| "gateway_storage_unavailable")?;
    if std::fs::rename(&pending, &path).is_err() {
        let _ = std::fs::remove_file(&pending);
        return Err("gateway_storage_unavailable");
    }
    Ok(())
}

fn load_sync_state(app: &AppHandle) -> PersistedSyncState {
    let Ok(path) = sync_state_path(app) else {
        return PersistedSyncState::default();
    };
    let Ok(bytes) = read_small_file(&path, MAX_STATE_BYTES) else {
        return PersistedSyncState::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

fn save_sync_state(app: &AppHandle, state: &PersistedSyncState) -> Result<(), &'static str> {
    let path = sync_state_path(app)?;
    let pending = path.with_extension("json.pending");
    let bytes = serde_json::to_vec_pretty(state).map_err(|_| "gateway_storage_invalid")?;
    std::fs::write(&pending, bytes).map_err(|_| "gateway_storage_unavailable")?;
    if std::fs::rename(&pending, &path).is_err() {
        let _ = std::fs::remove_file(&pending);
        return Err("gateway_storage_unavailable");
    }
    Ok(())
}

fn build_view(app: &AppHandle) -> CloudGatewayView {
    let settings = get_settings(app);
    let url = validate_gateway_url(&settings.gateway_url).unwrap_or_default();
    let token_configured = load_token(app).is_ok();
    let gateway_ready = !url.is_empty() && token_configured;
    let effective_mode =
        if settings.recognition_mode == RecognitionMode::CloudFirst && gateway_ready {
            RecognitionMode::CloudFirst
        } else {
            RecognitionMode::LocalOnly
        };
    let sync = load_sync_state(app);
    CloudGatewayView {
        recognition_mode: settings.recognition_mode,
        gateway_url: settings.gateway_url,
        auto_sync_vocabulary: settings.gateway_auto_sync_vocabulary,
        token_configured,
        gateway_ready,
        effective_mode,
        last_synced_revision: sync.last_synced_revision,
        last_synced_at_ms: sync.last_synced_at_ms,
        synced_active_count: sync.synced_active_count,
    }
}

/// Returns `true` when the app is in `cloud_first` mode AND both `gateway_url`
/// and DPAPI `gateway_token` are configured — so startup skips preloading local
/// `SenseVoiceSmall` (`0 MB` resident memory) and idle watcher unloads fallback
/// models after 5 minutes of local inactivity.
pub fn should_defer_startup_model_load(app: &AppHandle) -> bool {
    let Ok(_lock) = GATEWAY_IO.lock() else {
        return false;
    };
    build_view(app).effective_mode == RecognitionMode::CloudFirst
}

pub fn should_auto_sync_on_startup(app: &AppHandle) -> bool {
    let Ok(_lock) = GATEWAY_IO.lock() else {
        return false;
    };
    let view = build_view(app);
    view.gateway_ready && view.auto_sync_vocabulary
}

#[tauri::command]
#[specta::specta]
pub fn get_cloud_gateway_config(app: AppHandle) -> Result<CloudGatewayView, String> {
    let _lock = GATEWAY_IO
        .lock()
        .map_err(|_| "gateway_storage_unavailable")?;
    Ok(build_view(&app))
}

#[tauri::command]
#[specta::specta]
pub fn save_cloud_gateway_config(
    app: AppHandle,
    config: CloudGatewayConfig,
) -> Result<CloudGatewayView, String> {
    let cleaned_url = validate_gateway_url(&config.gateway_url).map_err(str::to_string)?;
    let view = {
        let _lock = GATEWAY_IO
            .lock()
            .map_err(|_| "gateway_storage_unavailable")?;
        let mut settings = get_settings(&app);
        settings.recognition_mode = config.recognition_mode;
        settings.gateway_url = cleaned_url;
        settings.gateway_auto_sync_vocabulary = config.auto_sync_vocabulary;
        write_settings(&app, settings);
        build_view(&app)
    };

    if view.effective_mode == RecognitionMode::LocalOnly {
        if let Some(tm) = app.try_state::<Arc<TranscriptionManager>>() {
            tm.initiate_model_load();
        }
    } else if view.auto_sync_vocabulary {
        schedule_auto_vocabulary_sync(&app);
    }
    let _ = app.emit("cloud-gateway-updated", &view);
    Ok(view)
}

#[tauri::command]
#[specta::specta]
pub fn set_cloud_gateway_token(app: AppHandle, token: String) -> Result<CloudGatewayView, String> {
    let secret = token_from_string(token).map_err(str::to_string)?;
    let view = {
        let _lock = GATEWAY_IO
            .lock()
            .map_err(|_| "gateway_storage_unavailable")?;
        persist_token(&app, secret).map_err(str::to_string)?;
        build_view(&app)
    };
    if view.gateway_ready && view.auto_sync_vocabulary {
        schedule_auto_vocabulary_sync(&app);
    }
    let _ = app.emit("cloud-gateway-updated", &view);
    Ok(view)
}

#[tauri::command]
#[specta::specta]
pub fn clear_cloud_gateway_token(app: AppHandle) -> Result<CloudGatewayView, String> {
    let view = {
        let _lock = GATEWAY_IO
            .lock()
            .map_err(|_| "gateway_storage_unavailable")?;
        #[cfg(target_os = "macos")]
        crate::keychain::clear(crate::keychain::GATEWAY)
            .map_err(|_| "gateway_token_unavailable")?;
        #[cfg(not(target_os = "macos"))]
        {
            let path = token_storage_path(&app).map_err(str::to_string)?;
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err("gateway_storage_unavailable".into()),
            }
        }
        build_view(&app)
    };
    if let Some(tm) = app.try_state::<Arc<TranscriptionManager>>() {
        tm.initiate_model_load();
    }
    let _ = app.emit("cloud-gateway-updated", &view);
    Ok(view)
}

fn bearer_header(token: &SecretBytes) -> Result<reqwest::header::HeaderValue, &'static str> {
    let mut raw = Vec::with_capacity(7 + token.0.len());
    raw.extend_from_slice(b"Bearer ");
    raw.extend_from_slice(&token.0);
    let result =
        reqwest::header::HeaderValue::from_bytes(&raw).map_err(|_| "invalid_gateway_token");
    for b in &mut raw {
        unsafe { std::ptr::write_volatile(b, 0) };
    }
    std::sync::atomic::compiler_fence(Ordering::SeqCst);
    let mut header = result?;
    header.set_sensitive(true);
    Ok(header)
}

#[tauri::command]
#[specta::specta]
pub async fn test_cloud_gateway_connection(
    app: AppHandle,
    gateway_url: Option<String>,
) -> Result<GatewayConnectionTestResult, String> {
    let (url, token) = {
        let _lock = GATEWAY_IO
            .lock()
            .map_err(|_| "gateway_storage_unavailable")?;
        let candidate_url = gateway_url.unwrap_or_else(|| get_settings(&app).gateway_url);
        let url = validate_gateway_url(&candidate_url).map_err(str::to_string)?;
        if url.is_empty() {
            return Err("missing_gateway_url".into());
        }
        let token = load_token(&app).map_err(str::to_string)?;
        (url, token)
    };

    let client = GATEWAY_HTTP.as_ref().map_err(|code| (*code).to_string())?;
    let auth = bearer_header(&token).map_err(str::to_string)?;
    let started = Instant::now();
    let response = client
        .get(format!("{url}/v1/status"))
        .header(reqwest::header::AUTHORIZATION, auth)
        .timeout(Duration::from_secs(6))
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                "gateway_timeout".to_string()
            } else {
                "gateway_unreachable".to_string()
            }
        })?;
    let latency_ms = started.elapsed().as_millis().min(u64::MAX as u128) as u64;

    if response.status() == reqwest::StatusCode::UNAUTHORIZED
        || response.status() == reqwest::StatusCode::FORBIDDEN
    {
        return Ok(GatewayConnectionTestResult {
            reachable: true,
            authenticated: false,
            device_id: None,
            asr_ready: false,
            cleanup_ready: false,
            asr_model: None,
            cleanup_model: None,
            vocabulary_revision: 0,
            vocabulary_count: 0,
            latency_ms,
        });
    }
    if !response.status().is_success() {
        return Err("gateway_http_error".into());
    }

    let status: WireStatusResponse = response
        .json()
        .await
        .map_err(|_| "gateway_invalid_response".to_string())?;
    Ok(GatewayConnectionTestResult {
        reachable: true,
        authenticated: true,
        device_id: Some(status.device_id),
        asr_ready: status.asr_ready,
        cleanup_ready: status.cleanup_ready,
        asr_model: Some(status.asr_model),
        cleanup_model: Some(status.cleanup_model),
        vocabulary_revision: status.vocabulary_revision,
        vocabulary_count: status.vocabulary_count,
        latency_ms,
    })
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(1)
}

#[tauri::command]
#[specta::specta]
pub async fn sync_cloud_gateway_vocabulary(
    app: AppHandle,
    full_sync: bool,
) -> Result<GatewayVocabularySyncResult, String> {
    perform_vocabulary_sync(&app, full_sync).await
}

pub fn schedule_auto_vocabulary_sync(app: &AppHandle) {
    AUTO_SYNC_PENDING.store(true, Ordering::Release);
    if AUTO_SYNC_RUNNING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            AUTO_SYNC_PENDING.store(false, Ordering::Release);
            // Small debounce so bursts of local edits coalesce into a single sync request.
            tokio::time::sleep(Duration::from_millis(600)).await;
            let should_run = {
                let Ok(_lock) = GATEWAY_IO.lock() else {
                    break;
                };
                let view = build_view(&app);
                view.gateway_ready && view.auto_sync_vocabulary
            };
            if should_run {
                if let Err(err) = perform_vocabulary_sync(&app, false).await {
                    log::debug!("Background gateway vocabulary sync skipped/failed: {err}");
                }
            }
            if !AUTO_SYNC_PENDING.load(Ordering::Acquire) {
                AUTO_SYNC_RUNNING.store(false, Ordering::Release);
                break;
            }
        }
    });
}

async fn perform_vocabulary_sync(
    app: &AppHandle,
    full_sync: bool,
) -> Result<GatewayVocabularySyncResult, String> {
    let (url, token, prev_state, custom_words) = {
        let _lock = GATEWAY_IO
            .lock()
            .map_err(|_| "gateway_storage_unavailable")?;
        let settings = get_settings(app);
        let url = validate_gateway_url(&settings.gateway_url).map_err(str::to_string)?;
        if url.is_empty() {
            return Err("missing_gateway_url".into());
        }
        let token = load_token(app).map_err(str::to_string)?;
        let prev_state = load_sync_state(app);
        (url, token, prev_state, settings.custom_words)
    };

    let app_for_db = app.clone();
    let snapshot = tauri::async_runtime::spawn_blocking(move || {
        crate::vocabulary::export_sync_snapshot(&app_for_db)
    })
    .await
    .map_err(|_| "vocabulary_unavailable".to_string())??;

    let stamp = now_ms();
    let mut entries: Vec<WireVocabularyEntry> = Vec::new();

    for confirmed in snapshot.confirmed {
        entries.push(WireVocabularyEntry {
            term: confirmed.term,
            kind: "confirmed".into(),
            aliases: confirmed.aliases,
            context: confirmed.context,
            count: 2,
            enabled: confirmed.enabled,
            updated_at_ms: stamp,
            revision: 0,
        });
    }
    for word in custom_words {
        let trimmed = word.trim();
        if !trimmed.is_empty() {
            entries.push(WireVocabularyEntry {
                term: trimmed.to_string(),
                kind: "custom".into(),
                aliases: Vec::new(),
                context: None,
                count: 2,
                enabled: true,
                updated_at_ms: stamp,
                revision: 0,
            });
        }
    }
    for (term, count) in snapshot.liked_terms {
        entries.push(WireVocabularyEntry {
            term,
            kind: "liked".into(),
            aliases: Vec::new(),
            context: None,
            count,
            enabled: true,
            updated_at_ms: stamp,
            revision: 0,
        });
    }
    for (term, count) in snapshot.frequent_tech_terms {
        entries.push(WireVocabularyEntry {
            term,
            kind: "frequent".into(),
            aliases: Vec::new(),
            context: None,
            count,
            enabled: true,
            updated_at_ms: stamp,
            revision: 0,
        });
    }
    for deleted_term in snapshot.deleted_terms {
        entries.push(WireVocabularyEntry {
            term: deleted_term,
            kind: "frequent".into(),
            aliases: Vec::new(),
            context: None,
            count: 1,
            enabled: false,
            updated_at_ms: stamp,
            revision: 0,
        });
    }

    let uploaded_candidates = entries.len().min(u32::MAX as usize) as u32;
    let since_revision = if full_sync {
        0
    } else {
        prev_state.last_synced_revision
    };

    let client = GATEWAY_HTTP.as_ref().map_err(|code| (*code).to_string())?;
    let auth = bearer_header(&token).map_err(str::to_string)?;
    let response = client
        .post(format!("{url}/v1/vocabulary/sync"))
        .header(reqwest::header::AUTHORIZATION, auth)
        .json(&json!({
            "since_revision": since_revision,
            "entries": entries,
        }))
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                "gateway_timeout".to_string()
            } else {
                "gateway_unreachable".to_string()
            }
        })?;

    if response.status() == reqwest::StatusCode::UNAUTHORIZED
        || response.status() == reqwest::StatusCode::FORBIDDEN
    {
        return Err("gateway_unauthorized".into());
    }
    if !response.status().is_success() {
        return Err("gateway_http_error".into());
    }

    let sync_res: WireVocabularySyncResponse = response
        .json()
        .await
        .map_err(|_| "gateway_invalid_response".to_string())?;

    let imported_local = {
        let _lock = GATEWAY_IO
            .lock()
            .map_err(|_| "gateway_storage_unavailable")?;
        let mut next_state = if full_sync {
            PersistedSyncState::default()
        } else {
            load_sync_state(app)
        };
        let mut imported = 0u32;
        let mut settings = get_settings(app);
        let mut settings_changed = false;

        for remote in &sync_res.entries {
            next_state
                .synced_terms
                .retain(|t| !t.eq_ignore_ascii_case(&remote.term));
            if remote.enabled {
                if next_state.synced_terms.len() < 256 {
                    next_state.synced_terms.push(remote.term.clone());
                }
                // Merge remote custom/confirmed terms into local custom_words so
                // offline local SenseVoice / Whisper fallback also benefits.
                if matches!(remote.kind.as_str(), "confirmed" | "custom")
                    && settings.custom_words.len() < 200
                    && !settings
                        .custom_words
                        .iter()
                        .any(|w| w.eq_ignore_ascii_case(&remote.term))
                {
                    settings.custom_words.push(remote.term.clone());
                    settings_changed = true;
                    imported += 1;
                }
            }
        }
        if settings_changed {
            write_settings(app, settings);
        }
        next_state.last_synced_revision = sync_res.revision;
        next_state.last_synced_at_ms = Some(now_ms());
        next_state.synced_active_count = sync_res.total_active;
        save_sync_state(app, &next_state).map_err(str::to_string)?;
        let view = build_view(app);
        let _ = app.emit("cloud-gateway-updated", &view);
        imported
    };

    Ok(GatewayVocabularySyncResult {
        uploaded_candidates,
        upserted_remote: sync_res.upserted,
        imported_local,
        revision: sync_res.revision,
        total_active: sync_res.total_active,
    })
}

/// Snapshot captured at recording start so an in-flight recording uses a frozen
/// route and credentials even if settings are changed mid-utterance.
pub(crate) struct GatewaySession {
    active: Option<ActiveGatewaySession>,
    live: Option<live::Recording>,
}

struct ActiveGatewaySession {
    gateway_url: String,
    token: SecretBytes,
    language: String,
    cleanup_enabled: bool,
    strength: CleanupStrength,
    natural_paragraphs: bool,
}

impl GatewaySession {
    pub(crate) fn inactive() -> Self {
        Self {
            active: None,
            live: None,
        }
    }

    pub(crate) fn is_active(&self) -> bool {
        self.active.is_some()
    }

    pub(crate) fn start_live(&mut self, app: &AppHandle, route: Arc<AudioRoute>) {
        let Some(active) = &self.active else {
            return;
        };
        let Ok(authorization) = bearer_header(&active.token) else {
            return;
        };
        let metadata = DictateMetadata::from(active);
        let app = app.clone();
        self.live = Some(live::Recording::start(
            route,
            active.gateway_url.clone(),
            authorization,
            move || metadata.encode(&app),
        ));
    }
}

pub(crate) fn capture_session(app: &AppHandle, cleanup: &CleanupSession) -> GatewaySession {
    let Ok(_lock) = GATEWAY_IO.lock() else {
        return GatewaySession::inactive();
    };
    let settings = get_settings(app);
    if settings.recognition_mode != RecognitionMode::CloudFirst {
        return GatewaySession::inactive();
    }
    let Ok(gateway_url) = validate_gateway_url(&settings.gateway_url) else {
        return GatewaySession::inactive();
    };
    if gateway_url.is_empty() {
        return GatewaySession::inactive();
    }
    let Ok(token) = load_token(app) else {
        return GatewaySession::inactive();
    };
    let (cleanup_enabled, strength, natural_paragraphs) = cleanup.gateway_preferences();
    GatewaySession {
        active: Some(ActiveGatewaySession {
            gateway_url,
            token,
            language: settings.selected_language,
            cleanup_enabled,
            strength,
            natural_paragraphs,
        }),
        live: None,
    }
}

#[derive(Serialize)]
struct DictateMetadata {
    language: String,
    cleanup_enabled: bool,
    strength: &'static str,
    natural_paragraphs: bool,
    hint_terms: Vec<String>,
}

impl From<&ActiveGatewaySession> for DictateMetadata {
    fn from(active: &ActiveGatewaySession) -> Self {
        let strength = match active.strength {
            CleanupStrength::Faithful => "faithful",
            CleanupStrength::Light => "light",
            CleanupStrength::Clear => "clear",
        };
        Self {
            language: active.language.clone(),
            cleanup_enabled: active.cleanup_enabled,
            strength,
            natural_paragraphs: active.natural_paragraphs,
            hint_terms: Vec::new(),
        }
    }
}

impl DictateMetadata {
    fn encode(mut self, app: &AppHandle) -> Result<String, &'static str> {
        self.hint_terms = crate::vocabulary::active_hint_terms(app, 24);
        serde_json::to_string(&self).map_err(|_| "gateway_metadata_invalid")
    }
}

pub fn encode_wav_16k_mono_in_memory(samples: &[f32]) -> Result<Vec<u8>, &'static str> {
    if samples.is_empty() {
        return Err("empty_audio");
    }
    let spec = WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let mut cursor = Cursor::new(Vec::with_capacity(44 + samples.len() * 2));
    {
        let mut writer = WavWriter::new(&mut cursor, spec).map_err(|_| "wav_encode_failed")?;
        for &sample in samples {
            let clamped = if sample.is_finite() {
                sample.clamp(-1.0, 1.0)
            } else {
                0.0
            };
            let sample_i16 = (clamped * i16::MAX as f32) as i16;
            writer
                .write_sample(sample_i16)
                .map_err(|_| "wav_encode_failed")?;
        }
        writer.finalize().map_err(|_| "wav_encode_failed")?;
    }
    Ok(cursor.into_inner())
}

pub fn build_multipart_dictate_body(wav_bytes: &[u8], metadata_json: &str) -> (String, Vec<u8>) {
    let seq = MULTIPART_SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let boundary = format!("----QingshuGatewayBoundary{nanos:016x}{seq:016x}");
    let content_type = format!("multipart/form-data; boundary={boundary}");

    let mut body = Vec::with_capacity(wav_bytes.len() + metadata_json.len() + 512);
    body.extend_from_slice(b"--");
    body.extend_from_slice(boundary.as_bytes());
    body.extend_from_slice(
        b"\r\nContent-Disposition: form-data; name=\"metadata\"\r\nContent-Type: application/json; charset=utf-8\r\n\r\n",
    );
    body.extend_from_slice(metadata_json.as_bytes());
    body.extend_from_slice(b"\r\n--");
    body.extend_from_slice(boundary.as_bytes());
    body.extend_from_slice(
        b"\r\nContent-Disposition: form-data; name=\"audio\"; filename=\"dictation.wav\"\r\nContent-Type: audio/wav\r\n\r\n",
    );
    body.extend_from_slice(wav_bytes);
    body.extend_from_slice(b"\r\n--");
    body.extend_from_slice(boundary.as_bytes());
    body.extend_from_slice(b"--\r\n");

    (content_type, body)
}

pub fn dictate_timeout_ms(sample_count: usize) -> u64 {
    // 16 kHz mono: add 100 ms budget per second of audio beyond 20 s, capped at 25 s.
    let secs = (sample_count / 16_000) as u64;
    (BASE_DICTATE_TIMEOUT_MS + secs.saturating_sub(20).saturating_mul(150))
        .min(MAX_DICTATE_TIMEOUT_MS)
}

pub(crate) async fn dictate_with_session(
    app: &AppHandle,
    session: &mut GatewaySession,
    samples: &[f32],
) -> Result<crate::actions::ProcessedTranscription, &'static str> {
    let active = session.active.as_ref().ok_or("gateway_not_active")?;
    let timeout_ms = dictate_timeout_ms(samples.len());
    let budget = Duration::from_millis(timeout_ms);
    let started = Instant::now();
    if let Some(live) = session.live.take() {
        // Reserve at least half of the cloud budget for a batch retry if the
        // live socket stalls. Both paths still share one post-release budget.
        let live_budget = (budget / 2).min(MAX_LIVE_FINALIZE_TIMEOUT);
        match live.finish(live_budget).await {
            Ok(wire) => {
                log::debug!("Live dictation finalized");
                return Ok(process_dictate_response(app, active, wire, timeout_ms));
            }
            Err(reason) => {
                log::debug!("Live dictation unavailable ({reason}); checking batch fallback");
            }
        }
    }
    let wire = dictate_batch(
        app,
        active,
        samples,
        budget.saturating_sub(started.elapsed()),
    )
    .await?;
    Ok(process_dictate_response(app, active, wire, timeout_ms))
}

async fn dictate_batch(
    app: &AppHandle,
    active: &ActiveGatewaySession,
    samples: &[f32],
    budget: Duration,
) -> Result<WireDictateResponse, &'static str> {
    let started = Instant::now();
    let wav_bytes = encode_wav_16k_mono_in_memory(samples)?;
    let metadata_json = DictateMetadata::from(active).encode(app)?;

    let (content_type, body) = build_multipart_dictate_body(&wav_bytes, &metadata_json);
    drop(wav_bytes);

    let client = GATEWAY_HTTP.as_ref().map_err(|code| *code)?;
    let auth = bearer_header(&active.token)?;
    let remaining = budget.saturating_sub(started.elapsed());
    if remaining.is_zero() {
        return Err("gateway_timeout");
    }

    let mut response = client
        .post(format!("{}/v1/dictate", active.gateway_url))
        .header(reqwest::header::AUTHORIZATION, auth)
        .header(reqwest::header::CONTENT_TYPE, content_type)
        .body(body)
        .timeout(remaining)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                "gateway_timeout"
            } else {
                "gateway_unreachable"
            }
        })?;

    if !response.status().is_success() {
        return Err(match response.status().as_u16() {
            401 | 403 => "gateway_unauthorized",
            413 => "gateway_audio_too_large",
            500..=599 => "gateway_upstream_error",
            _ => "gateway_http_error",
        });
    }

    let mut resp_bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| {
        if e.is_timeout() {
            "gateway_timeout"
        } else {
            "gateway_unreachable"
        }
    })? {
        if resp_bytes.len().saturating_add(chunk.len()) > 2 * 1024 * 1024 {
            return Err("gateway_response_too_large");
        }
        resp_bytes.extend_from_slice(&chunk);
    }

    serde_json::from_slice(&resp_bytes).map_err(|_| "gateway_invalid_response")
}

fn process_dictate_response(
    app: &AppHandle,
    active: &ActiveGatewaySession,
    wire: WireDictateResponse,
    timeout_ms: u64,
) -> crate::actions::ProcessedTranscription {
    let prepared_raw = crate::vocabulary::apply_confirmed(app, &wire.raw_text);
    let (final_candidate, cleanup_outcome) = if wire.raw_text.trim().is_empty() {
        (String::new(), "off".to_string())
    } else if wire.cleanup_applied {
        // Run deterministic local safety guardrails against both raw and locally-aliased text.
        match dictation_cleanup::validate_cleanup_text(&wire.raw_text, &wire.formatted_text)
            .or_else(|_| {
                dictation_cleanup::validate_cleanup_text(&prepared_raw, &wire.formatted_text)
            }) {
            Ok(valid) => (valid, "cleaned".to_string()),
            Err(guard_reason) => (prepared_raw.clone(), guard_reason.to_string()),
        }
    } else if let Some(reason) = wire.fallback_reason {
        (prepared_raw.clone(), reason)
    } else {
        (
            prepared_raw.clone(),
            if active.cleanup_enabled {
                "cleaned".to_string()
            } else {
                "off".to_string()
            },
        )
    };

    let final_text = crate::dictation_lifecycle::normalize_output(&final_candidate);
    crate::actions::ProcessedTranscription {
        raw_text: wire.raw_text,
        final_text: final_text.clone(),
        post_processed_text: Some(final_text),
        post_process_prompt: None,
        post_process_requested: active.cleanup_enabled,
        cleanup_ms: wire.cleanup_ms,
        cleanup_budget_ms: timeout_ms,
        cleanup_outcome,
        gateway_asr_ms: Some(wire.asr_ms),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_gateway_urls_and_rejects_embedded_credentials_or_queries() {
        assert_eq!(validate_gateway_url("  ").unwrap(), "");
        assert_eq!(
            validate_gateway_url("https://dictation.astrmind.com/").unwrap(),
            "https://dictation.astrmind.com"
        );
        assert_eq!(
            validate_gateway_url("http://127.0.0.1:8787").unwrap(),
            "http://127.0.0.1:8787"
        );
        for bad in [
            "ftp://dictation.astrmind.com",
            "https://user:pass@dictation.astrmind.com",
            "https://dictation.astrmind.com?token=secret",
            "https://dictation.astrmind.com#frag",
            "https://dictation.astrmind.com:0",
            "https://dictation.astrmind.com:abc",
        ] {
            assert_eq!(
                validate_gateway_url(bad),
                Err("invalid_gateway_url"),
                "{bad}"
            );
        }
    }

    #[test]
    fn encodes_valid_16k_mono_wav_and_builds_rfc7578_multipart_in_memory() {
        let samples = vec![0.0f32, 0.25, -0.25, 1.5, -1.5, f32::NAN];
        let wav = encode_wav_16k_mono_in_memory(&samples).unwrap();
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        let reader = hound::WavReader::new(Cursor::new(&wav)).unwrap();
        assert_eq!(reader.spec().sample_rate, 16_000);
        assert_eq!(reader.spec().channels, 1);
        assert_eq!(reader.len() as usize, samples.len());

        let (ct, body) = build_multipart_dictate_body(&wav, r#"{"language":"auto"}"#);
        assert!(ct.starts_with("multipart/form-data; boundary=----QingshuGatewayBoundary"));
        assert!(body.windows(4).any(|w| w == b"RIFF"));
    }

    #[test]
    fn token_validation_enforces_printable_ascii_and_minimum_length() {
        assert!(token_from_string("  qsgw_valid_device_token_12345 \n".into()).is_ok());
        assert_eq!(
            token_from_string("short".into()).err(),
            Some("invalid_gateway_token")
        );
        assert_eq!(
            token_from_string("bad token with spaces inside".into()).err(),
            Some("invalid_gateway_token")
        );
    }
}
