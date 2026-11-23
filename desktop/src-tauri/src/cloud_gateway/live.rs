//! Live dictation audio is routed independently from the local recognizer.
use super::{WireDictateResponse, CONNECT_TIMEOUT_MS};
use crate::audio_toolkit::audio::FrameResampler;
use futures_util::{SinkExt, StreamExt};
use reqwest::header::{HeaderValue, AUTHORIZATION};
use serde::Deserialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::{net::TcpStream, sync::mpsc, time::timeout};
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{client::IntoClientRequest, protocol::WebSocketConfig, Error, Message},
    MaybeTlsStream, WebSocketStream,
};

// About four seconds at the recorder's normal cadence. An overflow closes
// this route so a network stall cannot silently drop part of a transcript.
const AUDIO_QUEUE_CAPACITY: usize = 128;
const AUDIO_FRAME_DURATION: Duration = Duration::from_millis(30);
const SEND_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

enum AudioCommand {
    Audio(Vec<f32>),
    Finish,
}

struct AudioInput {
    id: Arc<()>,
    sender: mpsc::Sender<AudioCommand>,
}

#[derive(Default)]
pub(crate) struct AudioRoute {
    input: Mutex<Option<AudioInput>>,
    open: AtomicBool,
}

impl AudioRoute {
    fn attach(&self, id: Arc<()>, sender: mpsc::Sender<AudioCommand>) {
        if let Ok(mut input) = self.input.lock() {
            *input = Some(AudioInput { id, sender });
            self.open.store(true, Ordering::Relaxed);
        }
    }

    fn take(&self, id: &Arc<()>) -> Option<mpsc::Sender<AudioCommand>> {
        let mut input = self.input.lock().ok()?;
        if !input.as_ref().is_some_and(|slot| Arc::ptr_eq(&slot.id, id)) {
            return None;
        }
        self.open.store(false, Ordering::Relaxed);
        input.take().map(|slot| slot.sender)
    }

    pub(crate) fn feed(&self, frame: &[f32]) {
        if !self.open.load(Ordering::Relaxed) {
            return;
        }
        let Ok(mut input) = self.input.lock() else {
            return;
        };
        if let Some(slot) = input.as_ref() {
            let accepted = slot.sender.try_reserve().map(|permit| {
                permit.send(AudioCommand::Audio(frame.to_vec()));
            });
            if accepted.is_err() {
                *input = None;
                self.open.store(false, Ordering::Relaxed);
            }
        }
    }
}

/// Owns the socket task and the sole route for this capture. Dropping it
/// closes the network stream on cancellation and on recording-start failures.
pub(super) struct Recording {
    id: Arc<()>,
    route: Arc<AudioRoute>,
    worker: tauri::async_runtime::JoinHandle<Result<WireDictateResponse, &'static str>>,
}

impl Recording {
    pub(super) fn start(
        route: Arc<AudioRoute>,
        gateway_url: String,
        authorization: HeaderValue,
        metadata: impl FnOnce() -> Result<String, &'static str> + Send + 'static,
    ) -> Self {
        let id = Arc::new(());
        let (tx, rx) = mpsc::channel(AUDIO_QUEUE_CAPACITY);
        route.attach(Arc::clone(&id), tx);
        let worker = tauri::async_runtime::spawn(async move {
            let metadata = tauri::async_runtime::spawn_blocking(metadata)
                .await
                .map_err(|_| "gateway_metadata_invalid")??;
            exchange(&gateway_url, authorization, metadata, rx).await
        });
        Self { id, route, worker }
    }

    pub(super) async fn finish(
        mut self,
        budget: Duration,
    ) -> Result<WireDictateResponse, &'static str> {
        // stop_recording has already flushed its final callback. Removing the
        // route now guarantees Finish follows every frame on the same queue.
        let sender = self.route.take(&self.id);
        timeout(budget, async {
            if let Some(sender) = sender {
                let _ = sender.send(AudioCommand::Finish).await;
            }
            (&mut self.worker)
                .await
                .map_err(|_| "gateway_stream_unavailable")?
        })
        .await
        .map_err(|_| "gateway_stream_timeout")?
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        self.route.take(&self.id);
        self.worker.abort();
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Reply {
    Notice(Notice),
    Final(WireDictateResponse),
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Notice {
    Ready,
    Partial,
    Error,
}

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

async fn exchange(
    gateway_url: &str,
    authorization: HeaderValue,
    metadata: String,
    mut audio: mpsc::Receiver<AudioCommand>,
) -> Result<WireDictateResponse, &'static str> {
    let (scheme, rest) = gateway_url.split_once("://").ok_or("invalid_gateway_url")?;
    let websocket_scheme = match scheme {
        "https" => "wss",
        "http" => "ws",
        _ => return Err("invalid_gateway_url"),
    };
    let mut request = format!("{websocket_scheme}://{rest}/v1/dictate/stream")
        .into_client_request()
        .map_err(|_| "invalid_gateway_url")?;
    request.headers_mut().insert(AUTHORIZATION, authorization);
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_RESPONSE_BYTES))
        .max_frame_size(Some(MAX_RESPONSE_BYTES));
    let (mut socket, _) = timeout(
        Duration::from_millis(CONNECT_TIMEOUT_MS),
        connect_async_with_config(request, Some(config), false),
    )
    .await
    .map_err(|_| "gateway_stream_connect_timeout")?
    .map_err(|error| match error {
        Error::Http(response) if matches!(response.status().as_u16(), 401 | 403) => {
            "gateway_unauthorized"
        }
        _ => "gateway_stream_unavailable",
    })?;

    send(&mut socket, Message::Text(metadata.into())).await?;
    let mut resampler = FrameResampler::new(16_000, 24_000, AUDIO_FRAME_DURATION);
    let mut finished = false;
    loop {
        tokio::select! {
            command = audio.recv(), if !finished => {
                let mut frames = Vec::new();
                match command {
                    Some(AudioCommand::Audio(samples)) => {
                        resampler.push(&samples, |frame| frames.push(pcm16(frame)));
                    }
                    Some(AudioCommand::Finish) => {
                        resampler.finish(|frame| frames.push(pcm16(frame)));
                        finished = true;
                    }
                    None => return Err("gateway_stream_audio_interrupted"),
                }
                for frame in frames {
                    send(&mut socket, frame).await?;
                }
                if finished {
                    send(&mut socket, Message::Text(r#"{"type":"finish"}"#.into())).await?;
                }
            }
            message = socket.next() => {
                match message {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str(&text).map_err(|_| "gateway_invalid_response")? {
                            Reply::Notice(Notice::Ready | Notice::Partial) => {}
                            Reply::Notice(Notice::Error) => return Err("gateway_stream_error"),
                            Reply::Final(result) if finished => return Ok(result),
                            Reply::Final(_) => return Err("gateway_stream_unexpected_final"),
                        }
                    }
                    Some(Ok(Message::Ping(_))) => {
                        // Tungstenite queues a pong when reading a ping.
                        timeout(SEND_TIMEOUT, socket.flush())
                            .await
                            .map_err(|_| "gateway_stream_timeout")?
                            .map_err(|_| "gateway_stream_disconnected")?;
                    }
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Binary(_) | Message::Frame(_))) => {
                        return Err("gateway_invalid_response");
                    }
                    _ => return Err("gateway_stream_disconnected"),
                }
            }
        }
    }
}

fn pcm16(samples: &[f32]) -> Message {
    let mut bytes = Vec::with_capacity(samples.len() * 2);
    for &sample in samples {
        let value = (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16;
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    Message::Binary(bytes.into())
}

async fn send(socket: &mut Socket, message: Message) -> Result<(), &'static str> {
    timeout(SEND_TIMEOUT, socket.send(message))
        .await
        .map_err(|_| "gateway_stream_timeout")?
        .map_err(|_| "gateway_stream_disconnected")
}

#[cfg(test)]
mod tests;
