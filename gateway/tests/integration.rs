//! End-to-end integration tests for `local-dictation-gateway` using local mock
//! OpenAI and Anthropic servers (zero external network calls, zero secrets).

use axum::extract::{Multipart, State as AxumState};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use local_dictation_gateway::auth::{SecretString, TokenRegistry};
use local_dictation_gateway::vocab::{
    VocabularyKind, VocabularyStore, VocabularySyncEntry, VocabularySyncRequest,
    VocabularySyncResponse,
};
use local_dictation_gateway::{
    build_router, AppState, DictateResponse, GatewayConfig, HealthResponse, StatusResponse,
};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::TcpListener;

#[derive(Default)]
struct MockUpstreamLog {
    openai_models: Vec<String>,
    openai_keywords: Vec<String>,
    openai_languages: Vec<String>,
    openai_prompts: Vec<String>,
    openai_singular_language: Vec<String>,
    anthropic_system_prompts: Vec<String>,
    anthropic_user_messages: Vec<String>,
    anthropic_delay_ms: u64,
    openai_reply_text: String,
    anthropic_reply_text: String,
}

async fn spawn_mock_upstreams(state: Arc<Mutex<MockUpstreamLog>>) -> String {
    let router = Router::new()
        .route(
            "/v1/audio/transcriptions",
            post(
                |AxumState(log): AxumState<Arc<Mutex<MockUpstreamLog>>>,
                 headers: HeaderMap,
                 mut mp: Multipart| async move {
                    let auth = headers
                        .get("authorization")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("");
                    if auth != "Bearer mock-openai-key-for-testing" {
                        return (
                            StatusCode::UNAUTHORIZED,
                            Json(json!({"error": "bad openai key"})),
                        );
                    }
                    let mut got_file = false;
                    while let Some(field) = mp.next_field().await.unwrap() {
                        let name = field.name().unwrap_or("").to_string();
                        match name.as_str() {
                            "file" => {
                                let bytes = field.bytes().await.unwrap();
                                got_file = !bytes.is_empty();
                            }
                            "model" => {
                                let val = field.text().await.unwrap();
                                log.lock().unwrap().openai_models.push(val);
                            }
                            "keywords[]" => {
                                let val = field.text().await.unwrap();
                                log.lock().unwrap().openai_keywords.push(val);
                            }
                            "languages[]" => {
                                let val = field.text().await.unwrap();
                                log.lock().unwrap().openai_languages.push(val);
                            }
                            "prompt" => {
                                let val = field.text().await.unwrap();
                                log.lock().unwrap().openai_prompts.push(val);
                            }
                            "language" => {
                                let val = field.text().await.unwrap();
                                log.lock().unwrap().openai_singular_language.push(val);
                            }
                            _ => {}
                        }
                    }
                    assert!(
                        got_file,
                        "OpenAI multipart must include non-empty audio file"
                    );
                    let reply = log.lock().unwrap().openai_reply_text.clone();
                    (StatusCode::OK, Json(json!({ "text": reply })))
                },
            ),
        )
        .route(
            "/v1/messages",
            post(
                |AxumState(log): AxumState<Arc<Mutex<MockUpstreamLog>>>,
                 headers: HeaderMap,
                 Json(body): Json<Value>| async move {
                    let key = headers
                        .get("x-api-key")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("");
                    if key != "mock-anthropic-key-for-testing" {
                        return (
                            StatusCode::UNAUTHORIZED,
                            Json(json!({"error": "bad anthropic key"})),
                        );
                    }
                    let (delay_ms, reply) = {
                        let mut guard = log.lock().unwrap();
                        if let Some(sys) = body.get("system").and_then(Value::as_str) {
                            guard.anthropic_system_prompts.push(sys.to_string());
                        }
                        if let Some(user_msg) = body
                            .get("messages")
                            .and_then(Value::as_array)
                            .and_then(|arr| arr.first())
                            .and_then(|m| m.get("content"))
                            .and_then(Value::as_str)
                        {
                            guard.anthropic_user_messages.push(user_msg.to_string());
                        }
                        (guard.anthropic_delay_ms, guard.anthropic_reply_text.clone())
                    };
                    if delay_ms > 0 {
                        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                    }
                    (
                        StatusCode::OK,
                        Json(json!({
                            "stop_reason": "end_turn",
                            "content": [{"type": "text", "text": reply}]
                        })),
                    )
                },
            ),
        )
        .with_state(state);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    format!("http://{addr}")
}

fn minimal_wav_bytes() -> Vec<u8> {
    // Valid 16 kHz mono 16-bit PCM WAV header + 320 samples (20 ms) of synthetic tone.
    let num_samples: u32 = 320;
    let data_bytes: u32 = num_samples * 2;
    let mut wav = Vec::with_capacity(44 + data_bytes as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&16_000u32.to_le_bytes());
    wav.extend_from_slice(&32_000u32.to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_bytes.to_le_bytes());
    for i in 0..num_samples {
        let sample: i16 = if i % 20 < 10 { 1200 } else { -1200 };
        wav.extend_from_slice(&sample.to_le_bytes());
    }
    wav
}

async fn spawn_gateway(upstream_base: &str, asr_model: &str, cleanup_timeout_ms: u64) -> String {
    let config = GatewayConfig {
        bind_addr: "127.0.0.1:0".into(),
        db_path: PathBuf::from(":memory:"),
        tokens: TokenRegistry::parse(
            "desktop=qsgw_desktop_test_token_12345,laptop=qsgw_laptop_test_token_67890",
        )
        .unwrap(),
        openai_api_key: SecretString::new("mock-openai-key-for-testing"),
        openai_base_url: format!("{upstream_base}/v1"),
        openai_asr_model: asr_model.to_string(),
        anthropic_api_key: SecretString::new("mock-anthropic-key-for-testing"),
        anthropic_base_url: upstream_base.to_string(),
        anthropic_cleanup_model: "claude-haiku-5-5".into(),
        max_audio_bytes: 1024 * 1024,
        asr_timeout_ms: 5_000,
        cleanup_timeout_ms,
    };
    let vocab = VocabularyStore::open_in_memory().unwrap();
    let state = AppState::new(config, vocab).unwrap();
    let app = build_router(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn end_to_end_auth_vocab_sync_dictate_multiline_and_cleanup_fallback() {
    let upstream_log = Arc::new(Mutex::new(MockUpstreamLog {
        openai_reply_text: "嗯今天做三件事 第一 把 SenseVoiceSmall 改成按需懒加载 第二 接入 Oracle 网关 第三 保持预算在 5 美元以内".into(),
        anthropic_reply_text: "今天做三件事：\n1. 把 SenseVoiceSmall 改成按需懒加载\n2. 接入 Oracle 网关\n3. 保持预算在 5 美元以内".into(),
        anthropic_delay_ms: 0,
        ..Default::default()
    }));
    let upstream_url = spawn_mock_upstreams(upstream_log.clone()).await;
    let gw_url = spawn_gateway(&upstream_url, "gpt-transcribe", 1_500).await;
    let client = reqwest::Client::new();

    // 1. /healthz works without auth and leaks nothing
    let health: HealthResponse = client
        .get(format!("{gw_url}/healthz"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(health.status, "ok");

    // 2. /v1/status rejects missing or wrong token, accepts desktop & laptop tokens without echoing keys
    assert_eq!(
        client
            .get(format!("{gw_url}/v1/status"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        client
            .get(format!("{gw_url}/v1/status"))
            .bearer_auth("qsgw_revoked_device_token_00000")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );

    let status_raw = client
        .get(format!("{gw_url}/v1/status"))
        .bearer_auth("qsgw_desktop_test_token_12345")
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(!status_raw.contains("mock-openai-key"));
    assert!(!status_raw.contains("mock-anthropic-key"));
    let status: StatusResponse = serde_json::from_str(&status_raw).unwrap();
    assert_eq!(status.device_id, "desktop");
    assert!(status.asr_ready);
    assert!(status.cleanup_ready);

    // 3. Desktop syncs technical vocabulary -> Laptop pulls incrementally -> Laptop dictates and gets Desktop's terms in ASR & Claude!
    let sync_res: VocabularySyncResponse = client
        .post(format!("{gw_url}/v1/vocabulary/sync"))
        .bearer_auth("qsgw_desktop_test_token_12345")
        .json(&VocabularySyncRequest {
            since_revision: 0,
            entries: vec![
                VocabularySyncEntry {
                    term: "SenseVoiceSmall".into(),
                    kind: VocabularyKind::Confirmed,
                    aliases: vec!["sense voice small".into()],
                    context: None,
                    count: 8,
                    enabled: true,
                    updated_at_ms: 1_700_000_000_000,
                    revision: 0,
                },
                VocabularySyncEntry {
                    term: "Claude Haiku".into(),
                    kind: VocabularyKind::Liked,
                    aliases: vec![],
                    context: None,
                    count: 4,
                    enabled: true,
                    updated_at_ms: 1_700_000_000_000,
                    revision: 0,
                },
            ],
        })
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(sync_res.revision, 2);
    assert_eq!(sync_res.total_active, 2);

    let laptop_vocab: VocabularySyncResponse = client
        .get(format!("{gw_url}/v1/vocabulary?since_revision=0"))
        .bearer_auth("qsgw_laptop_test_token_67890")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(laptop_vocab.entries.len(), 2);

    // 4. Laptop calls /v1/dictate with WAV audio + metadata
    let form = reqwest::multipart::Form::new()
        .part(
            "audio",
            reqwest::multipart::Part::bytes(minimal_wav_bytes())
                .file_name("clip.wav")
                .mime_str("audio/wav")
                .unwrap(),
        )
        .text(
            "metadata",
            serde_json::to_string(&json!({
                "language": "zh-CN",
                "cleanup_enabled": true,
                "strength": "light",
                "natural_paragraphs": true,
                "hint_terms": ["Tauri"]
            }))
            .unwrap(),
        );

    let dictate_res: DictateResponse = client
        .post(format!("{gw_url}/v1/dictate"))
        .bearer_auth("qsgw_laptop_test_token_67890")
        .multipart(form)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert!(dictate_res.cleanup_applied);
    assert_eq!(dictate_res.fallback_reason, None);
    assert_eq!(
        dictate_res.formatted_text,
        "今天做三件事：\n1. 把 SenseVoiceSmall 改成按需懒加载\n2. 接入 Oracle 网关\n3. 保持预算在 5 美元以内"
    );
    assert_eq!(dictate_res.vocabulary_revision, 2);

    {
        let log = upstream_log.lock().unwrap();
        assert_eq!(log.openai_models, vec!["gpt-transcribe"]);
        assert_eq!(
            log.openai_keywords,
            vec!["Tauri", "SenseVoiceSmall", "Claude Haiku"]
        );
        assert_eq!(log.openai_languages, vec!["zh", "en"]);
        assert!(log.openai_singular_language.is_empty());
        assert!(log.anthropic_system_prompts[0].contains("SenseVoiceSmall"));
    }

    // 5. Simulate Anthropic timeout -> graceful fallback returning raw_text!
    {
        let mut log = upstream_log.lock().unwrap();
        log.anthropic_delay_ms = 2_200; // exceeds 1_500ms gateway cleanup_timeout_ms
    }
    let fallback_form = reqwest::multipart::Form::new().part(
        "audio",
        reqwest::multipart::Part::bytes(minimal_wav_bytes())
            .file_name("clip.wav")
            .mime_str("audio/wav")
            .unwrap(),
    );
    let fallback_res: DictateResponse = client
        .post(format!("{gw_url}/v1/dictate"))
        .bearer_auth("qsgw_laptop_test_token_67890")
        .multipart(fallback_form)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(!fallback_res.cleanup_applied);
    assert_eq!(
        fallback_res.fallback_reason.as_deref(),
        Some("cleanup_timeout")
    );
    assert_eq!(
        fallback_res.formatted_text,
        "嗯今天做三件事 第一 把 SenseVoiceSmall 改成按需懒加载 第二 接入 Oracle 网关 第三 保持预算在 5 美元以内"
    );
}

#[tokio::test]
async fn compatibility_with_gpt_4o_mini_transcribe_multipart_parameters() {
    let upstream_log = Arc::new(Mutex::new(MockUpstreamLog {
        openai_reply_text: "请打开 Claude Code 运行 cargo test".into(),
        anthropic_reply_text: "请打开 Claude Code，运行 cargo test。".into(),
        ..Default::default()
    }));
    let upstream_url = spawn_mock_upstreams(upstream_log.clone()).await;
    let gw_url = spawn_gateway(&upstream_url, "gpt-4o-mini-transcribe", 3_000).await;
    let client = reqwest::Client::new();

    let form = reqwest::multipart::Form::new()
        .part(
            "audio",
            reqwest::multipart::Part::bytes(minimal_wav_bytes())
                .file_name("clip.wav")
                .mime_str("audio/wav")
                .unwrap(),
        )
        .text("language", "zh")
        .text("hint_terms", "Claude Code,rustls");

    let res: DictateResponse = client
        .post(format!("{gw_url}/v1/dictate"))
        .bearer_auth("qsgw_desktop_test_token_12345")
        .multipart(form)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert!(res.cleanup_applied);
    assert_eq!(res.formatted_text, "请打开 Claude Code，运行 cargo test。");
    let log = upstream_log.lock().unwrap();
    assert_eq!(log.openai_models, vec!["gpt-4o-mini-transcribe"]);
    assert!(log.openai_keywords.is_empty());
    assert!(log.openai_languages.is_empty());
    assert_eq!(log.openai_singular_language, vec!["zh"]);
    assert_eq!(log.openai_prompts, vec!["Claude Code, rustls"]);
}

#[tokio::test]
async fn guardrail_tampering_ogg_opus_oversize_and_tombstone_sync_coverage() {
    let upstream_log = Arc::new(Mutex::new(MockUpstreamLog {
        // Upstream Anthropic tampers budget 3500 -> 4500 inside a spoken list!
        openai_reply_text: "第一预算 3500 元 第二周四上线".into(),
        anthropic_reply_text: "1. 预算 4500 元\n2. 周四上线".into(),
        ..Default::default()
    }));
    let upstream_url = spawn_mock_upstreams(upstream_log).await;
    let gw_url = spawn_gateway(&upstream_url, "gpt-transcribe", 3_000).await;
    let client = reqwest::Client::new();

    // OGG/Opus magic header ("OggS...") accepted purely in memory; tampered digits caught by gateway guardrail!
    let mut ogg_bytes = b"OggS\x00\x02\x00\x00\x00\x00\x00\x00".to_vec();
    ogg_bytes.extend_from_slice(&[0u8; 64]);
    let ogg_form = reqwest::multipart::Form::new().part(
        "audio",
        reqwest::multipart::Part::bytes(ogg_bytes)
            .file_name("voice.opus")
            .mime_str("audio/ogg")
            .unwrap(),
    );
    let res: DictateResponse = client
        .post(format!("{gw_url}/v1/dictate"))
        .bearer_auth("qsgw_desktop_test_token_12345")
        .multipart(ogg_form)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(!res.cleanup_applied);
    assert_eq!(res.fallback_reason.as_deref(), Some("digits_changed"));
    assert_eq!(res.formatted_text, "第一预算 3500 元 第二周四上线");

    // Unsupported random binary rejected with 400
    let bad_form = reqwest::multipart::Form::new().part(
        "audio",
        reqwest::multipart::Part::bytes(vec![1, 2, 3, 4, 5])
            .file_name("bad.bin")
            .mime_str("application/octet-stream")
            .unwrap(),
    );
    assert_eq!(
        client
            .post(format!("{gw_url}/v1/dictate"))
            .bearer_auth("qsgw_desktop_test_token_12345")
            .multipart(bad_form)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );

    // Oversize audio (> 1 MiB test cap) rejected with 413
    let huge_form = reqwest::multipart::Form::new().part(
        "audio",
        reqwest::multipart::Part::bytes(vec![0u8; 1024 * 1024 + 64])
            .file_name("huge.wav")
            .mime_str("audio/wav")
            .unwrap(),
    );
    assert_eq!(
        client
            .post(format!("{gw_url}/v1/dictate"))
            .bearer_auth("qsgw_desktop_test_token_12345")
            .multipart(huge_form)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );

    // Tombstone (`enabled = false` at newer timestamp) deactivates synced term across devices
    let add_res: VocabularySyncResponse = client
        .post(format!("{gw_url}/v1/vocabulary/sync"))
        .bearer_auth("qsgw_desktop_test_token_12345")
        .json(&VocabularySyncRequest {
            since_revision: 0,
            entries: vec![VocabularySyncEntry {
                term: "ObsoleteTerm".into(),
                kind: VocabularyKind::Liked,
                aliases: vec![],
                context: None,
                count: 2,
                enabled: true,
                updated_at_ms: 10_000,
                revision: 0,
            }],
        })
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(add_res.total_active, 1);

    let del_res: VocabularySyncResponse = client
        .post(format!("{gw_url}/v1/vocabulary/sync"))
        .bearer_auth("qsgw_laptop_test_token_67890")
        .json(&VocabularySyncRequest {
            since_revision: add_res.revision,
            entries: vec![VocabularySyncEntry {
                term: "ObsoleteTerm".into(),
                kind: VocabularyKind::Liked,
                aliases: vec![],
                context: None,
                count: 2,
                enabled: false,
                updated_at_ms: 20_000,
                revision: 0,
            }],
        })
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(del_res.total_active, 0);
    assert_eq!(del_res.entries.len(), 1);
    assert!(!del_res.entries[0].enabled);
}
