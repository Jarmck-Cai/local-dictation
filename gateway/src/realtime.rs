//! Relays one authenticated push-to-talk recording to OpenAI Realtime.

use crate::{authenticate, upstream, ApiError, AppState, DictateContext, DictateMetadata};
use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::time::{timeout, Instant};
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{
        client::IntoClientRequest, protocol::WebSocketConfig, Message as UpstreamMessage,
    },
    MaybeTlsStream, WebSocketStream,
};

const MAX_CLIENT_MESSAGE: usize = 64 * 1024;
const MAX_STARTUP_AUDIO_BYTES: usize = 256 * 1024;
const MAX_TRANSCRIPT_BYTES: usize = 256 * 1024;
const MAX_PENDING_TRANSCRIPTS: usize = 4;
const MIN_COMMIT_BYTES: usize = 4_800; // 100 ms of 24 kHz mono PCM16
const STREAM_LIFETIME: Duration = Duration::from_secs(600);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(4);
const WRITE_TIMEOUT: Duration = Duration::from_secs(2);
const PREVIEW_INTERVAL: Duration = Duration::from_millis(120);
const CLIENT_DISCONNECTED: &str = "stream_client_disconnected";

type RealtimeSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;
type StreamResult<T> = Result<T, &'static str>;

pub(super) async fn handle_stream(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let device_id = authenticate(&state, &headers)?.to_owned();
    if state.config.openai_realtime_asr_model.is_none() || state.config.openai_api_key.is_none() {
        return Err(ApiError {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: "realtime_asr_not_configured",
        });
    }
    Ok(ws
        .read_buffer_size(8 * 1024)
        .max_message_size(MAX_CLIENT_MESSAGE)
        .max_frame_size(MAX_CLIENT_MESSAGE)
        .on_upgrade(move |mut socket| async move {
            let result = timeout(STREAM_LIFETIME, run(&mut socket, &state, device_id))
                .await
                .unwrap_or(Err("stream_session_timeout"));
            if let Err(code) = result {
                if code != CLIENT_DISCONNECTED {
                    log::warn!("dictate_stream_failed reason={code}");
                    let _ = send_client(&mut socket, &json!({"type": "error", "code": code})).await;
                }
            }
        })
        .into_response())
}

async fn run(socket: &mut WebSocket, state: &AppState, device_id: String) -> StreamResult<()> {
    let metadata = match timeout(CONNECT_TIMEOUT, read_client(socket)).await {
        Ok(Ok(Message::Text(text))) => {
            serde_json::from_str::<DictateMetadata>(&text).map_err(|_| "invalid_metadata_json")?
        }
        Ok(Err(code)) => return Err(code),
        Ok(Ok(_)) => return Err("missing_stream_metadata"),
        Err(_) => return Err("stream_metadata_timeout"),
    };
    let context = DictateContext::new(state, device_id, metadata);
    send_client(socket, &json!({"type": "ready"})).await?;
    let (raw, audio_len, asr_ms) = transcribe(socket, state, &context).await?;
    let response = while_client_open(socket, async {
        Ok(context.complete(state, raw, audio_len, asr_ms).await)
    })
    .await?;
    send_client(socket, &response).await
}

enum ClientFrame {
    Audio(Bytes),
    Finish,
}

async fn read_client(socket: &mut WebSocket) -> StreamResult<Message> {
    loop {
        match socket.recv().await {
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
            Some(Ok(Message::Close(_))) | Some(Err(_)) | None => return Err(CLIENT_DISCONNECTED),
            Some(Ok(message)) => return Ok(message),
        }
    }
}

async fn next_frame(socket: &mut WebSocket) -> StreamResult<ClientFrame> {
    match read_client(socket).await? {
        Message::Binary(frame) if !frame.is_empty() && frame.len() % 2 == 0 => {
            Ok(ClientFrame::Audio(frame))
        }
        Message::Binary(_) => Err("invalid_pcm_frame"),
        Message::Text(text) => {
            let payload: Value =
                serde_json::from_str(&text).map_err(|_| "invalid_stream_control")?;
            if payload.get("type").and_then(Value::as_str) == Some("finish") {
                Ok(ClientFrame::Finish)
            } else {
                Err("invalid_stream_control")
            }
        }
        _ => Err("invalid_stream_control"),
    }
}

async fn while_client_open<T>(
    socket: &mut WebSocket,
    operation: impl Future<Output = StreamResult<T>>,
) -> StreamResult<T> {
    tokio::select! {
        biased;
        message = read_client(socket) => Err(message.err().unwrap_or("stream_message_after_finish")),
        result = operation => result,
    }
}

async fn send_client<T: Serialize>(socket: &mut WebSocket, message: &T) -> StreamResult<()> {
    let message = serde_json::to_string(message).map_err(|_| "invalid_stream_response")?;
    timeout(WRITE_TIMEOUT, socket.send(Message::Text(message.into())))
        .await
        .map_err(|_| "stream_write_timeout")?
        .map_err(|_| CLIENT_DISCONNECTED)
}

fn account_audio(total: &mut usize, bytes: usize, limit: usize) -> StreamResult<()> {
    *total = total.saturating_add(bytes);
    if *total > limit {
        return Err("audio_too_large");
    }
    Ok(())
}

async fn transcribe(
    socket: &mut WebSocket,
    state: &AppState,
    context: &DictateContext,
) -> StreamResult<(String, usize, u64)> {
    let limit = state.config.max_audio_bytes;
    let mut audio_len = 0;
    let first = loop {
        match next_frame(socket).await? {
            ClientFrame::Finish => return Ok((String::new(), audio_len, 0)),
            ClientFrame::Audio(frame) => {
                account_audio(&mut audio_len, frame.len(), limit)?;
                // Avoid opening an upstream session for empty or digital-silence
                // recordings. Do not trim audio once real input has arrived.
                if frame.iter().any(|byte| *byte != 0) {
                    break frame;
                }
            }
        }
    };

    let mut startup_bytes = first.len();
    let mut startup_frames = vec![first];
    let mut finished_at = None;
    let connect = timeout(CONNECT_TIMEOUT, open_upstream(state, context));
    tokio::pin!(connect);
    let mut upstream = loop {
        tokio::select! {
            opened = &mut connect => break opened.map_err(|_| "asr_connect_timeout")??,
            frame = next_frame(socket) => {
                let frame = frame?;
                if finished_at.is_some() {
                    return Err("stream_message_after_finish");
                }
                match frame {
                    ClientFrame::Audio(frame) => {
                        account_audio(&mut audio_len, frame.len(), limit)?;
                        account_audio(&mut startup_bytes, frame.len(), MAX_STARTUP_AUDIO_BYTES)
                            .map_err(|_| "asr_startup_buffer_full")?;
                        startup_frames.push(frame);
                    }
                    ClientFrame::Finish => finished_at = Some(Instant::now()),
                }
            }
        }
    };

    let mut sent_bytes = startup_bytes;
    for frame in startup_frames {
        append_audio(&mut upstream, &frame).await?;
    }
    let mut preview = Preview::default();
    let finish_started = loop {
        if let Some(at) = finished_at {
            break at;
        }
        tokio::select! {
            frame = next_frame(socket) => match frame? {
                ClientFrame::Audio(frame) => {
                    account_audio(&mut audio_len, frame.len(), limit)?;
                    sent_bytes += frame.len();
                    append_audio(&mut upstream, &frame).await?;
                }
                ClientFrame::Finish => break Instant::now(),
            },
            event = read_upstream(&mut upstream) => {
                if let RealtimeEvent::Delta { item_id, delta } = event? {
                    preview.add(socket, item_id, &delta).await?;
                }
            }
        }
    };

    let finalize = async {
        if sent_bytes < MIN_COMMIT_BYTES {
            let padding = vec![0; MIN_COMMIT_BYTES - sent_bytes];
            append_audio(&mut upstream, &padding).await?;
        }
        send_upstream(&mut upstream, &json!({"type": "input_audio_buffer.commit"})).await?;
        await_transcript(&mut upstream).await
    };
    let budget = Duration::from_millis(state.config.asr_timeout_ms);
    let remaining = budget
        .checked_sub(finish_started.elapsed())
        .ok_or("asr_timeout")?;
    let raw = while_client_open(socket, async {
        timeout(remaining, finalize)
            .await
            .map_err(|_| "asr_timeout")?
    })
    .await?;
    Ok((raw, audio_len, finish_started.elapsed().as_millis() as u64))
}

#[derive(Default)]
struct Preview {
    item_id: Option<String>,
    text: String,
    last_sent: Option<Instant>,
}

impl Preview {
    async fn add(
        &mut self,
        socket: &mut WebSocket,
        item_id: String,
        delta: &str,
    ) -> StreamResult<()> {
        if delta.is_empty() || *self.item_id.get_or_insert_with(|| item_id.clone()) != item_id {
            return Ok(());
        }
        if self.text.len().saturating_add(delta.len()) > MAX_TRANSCRIPT_BYTES {
            return Err("asr_transcript_too_large");
        }
        self.text.push_str(delta);
        if self
            .last_sent
            .is_none_or(|last| last.elapsed() >= PREVIEW_INTERVAL)
        {
            send_client(socket, &json!({"type": "partial", "text": self.text})).await?;
            self.last_sent = Some(Instant::now());
        }
        Ok(())
    }
}

fn upstream_endpoint(base_url: &str) -> StreamResult<String> {
    let mut url = reqwest::Url::parse(base_url.trim()).map_err(|_| "invalid_realtime_url")?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("invalid_realtime_url");
    }
    let scheme = match url.scheme() {
        "https" => "wss",
        "http"
            if url.host_str().is_some_and(|host| {
                host.eq_ignore_ascii_case("localhost")
                    || host
                        .trim_matches(['[', ']'])
                        .parse::<std::net::IpAddr>()
                        .is_ok_and(|ip| ip.is_loopback())
            }) =>
        {
            "ws"
        }
        _ => return Err("invalid_realtime_url"),
    };
    url.set_scheme(scheme).map_err(|_| "invalid_realtime_url")?;
    Ok(format!(
        "{}/realtime?intent=transcription",
        url.as_str().trim_end_matches('/')
    ))
}

async fn open_upstream(state: &AppState, context: &DictateContext) -> StreamResult<RealtimeSocket> {
    let key = state
        .config
        .openai_api_key
        .as_ref()
        .ok_or("asr_not_configured")?;
    let model = state
        .config
        .openai_realtime_asr_model
        .as_deref()
        .ok_or("asr_not_configured")?;
    let mut request = upstream_endpoint(&state.config.openai_base_url)?
        .into_client_request()
        .map_err(|_| "invalid_realtime_url")?;
    let mut auth = HeaderValue::from_bytes(&[b"Bearer ".as_slice(), key.as_bytes()].concat())
        .map_err(|_| "invalid_openai_api_key")?;
    auth.set_sensitive(true);
    request.headers_mut().insert(header::AUTHORIZATION, auth);

    // tungstenite performs a single handshake without following redirects.
    let config = WebSocketConfig::default()
        .read_buffer_size(8 * 1024)
        .max_frame_size(Some(2 * MAX_TRANSCRIPT_BYTES))
        .max_message_size(Some(2 * MAX_TRANSCRIPT_BYTES));
    let (mut socket, _) = connect_async_with_config(request, Some(config), true)
        .await
        .map_err(|_| "asr_connect_failed")?;
    let mut transcription = json!({"model": model});
    let keywords: Vec<&str> = context
        .prompt_terms
        .iter()
        .map(String::as_str)
        .take(upstream::MAX_KEYWORDS_COUNT)
        .collect();
    if !keywords.is_empty() {
        transcription["keywords"] = json!(keywords);
    }
    if let Some(language) = upstream::normalize_iso639_1(context.meta.language.as_deref()) {
        transcription["languages"] = if language == "zh" {
            json!(["zh", "en"])
        } else {
            json!([language])
        };
    }
    send_upstream(
        &mut socket,
        &json!({
            "type": "session.update",
            "session": {
                "type": "transcription",
                "audio": {
                    "input": {
                        "format": {"type": "audio/pcm", "rate": 24000},
                        "transcription": transcription,
                        "turn_detection": null
                    }
                }
            }
        }),
    )
    .await?;
    // Only forward audio after OpenAI accepts the selected configuration.
    // The caller bounds connection and this acknowledgement together.
    loop {
        if matches!(
            read_upstream(&mut socket).await?,
            RealtimeEvent::SessionUpdated
        ) {
            return Ok(socket);
        }
    }
}

async fn send_upstream(socket: &mut RealtimeSocket, payload: &Value) -> StreamResult<()> {
    timeout(
        WRITE_TIMEOUT,
        socket.send(UpstreamMessage::Text(payload.to_string().into())),
    )
    .await
    .map_err(|_| "asr_write_timeout")?
    .map_err(|_| "asr_connection_lost")
}

async fn append_audio(socket: &mut RealtimeSocket, bytes: &[u8]) -> StreamResult<()> {
    send_upstream(
        socket,
        &json!({
            "type": "input_audio_buffer.append",
            "audio": STANDARD.encode(bytes),
        }),
    )
    .await
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum RealtimeEvent {
    #[serde(rename = "session.updated")]
    SessionUpdated,
    #[serde(rename = "conversation.item.input_audio_transcription.delta")]
    Delta { item_id: String, delta: String },
    #[serde(rename = "input_audio_buffer.committed")]
    Committed { item_id: String },
    #[serde(rename = "conversation.item.input_audio_transcription.completed")]
    Completed { item_id: String, transcript: String },
    #[serde(rename = "conversation.item.input_audio_transcription.failed")]
    Failed,
    #[serde(rename = "error")]
    Error,
    #[serde(other)]
    Other,
}

async fn read_upstream(socket: &mut RealtimeSocket) -> StreamResult<RealtimeEvent> {
    loop {
        match socket.next().await {
            Some(Ok(UpstreamMessage::Text(text))) => {
                let event = serde_json::from_str(&text).map_err(|_| "asr_invalid_response")?;
                return match event {
                    RealtimeEvent::Failed | RealtimeEvent::Error => {
                        Err("asr_realtime_upstream_error")
                    }
                    event => Ok(event),
                };
            }
            Some(Ok(UpstreamMessage::Ping(_) | UpstreamMessage::Pong(_))) => {}
            Some(Ok(UpstreamMessage::Close(_))) | Some(Err(_)) | None => {
                return Err("asr_connection_lost");
            }
            _ => return Err("asr_invalid_response"),
        }
    }
}

async fn await_transcript(socket: &mut RealtimeSocket) -> StreamResult<String> {
    let mut committed_id = None;
    let mut pending = HashMap::<String, String>::new();
    loop {
        match read_upstream(socket).await? {
            RealtimeEvent::Committed { item_id } if committed_id.is_none() => {
                if let Some(transcript) = pending.remove(&item_id) {
                    return Ok(transcript.trim().to_owned());
                }
                pending.clear();
                committed_id = Some(item_id);
            }
            RealtimeEvent::Completed {
                item_id,
                transcript,
            } => {
                if transcript.len() > MAX_TRANSCRIPT_BYTES {
                    return Err("asr_transcript_too_large");
                }
                match &committed_id {
                    Some(committed) if committed == &item_id => {
                        return Ok(transcript.trim().to_owned());
                    }
                    None => {
                        if pending.len() >= MAX_PENDING_TRANSCRIPTS
                            && !pending.contains_key(&item_id)
                        {
                            return Err("asr_unexpected_transcript_items");
                        }
                        pending.insert(item_id, transcript);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}
