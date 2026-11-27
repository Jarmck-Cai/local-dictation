use super::*;
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::{net::TcpListener, sync::oneshot, time::timeout};
use tokio_tungstenite::{accept_async, accept_hdr_async, tungstenite::Message};

const WAIT: Duration = Duration::from_secs(3);

#[tokio::test]
async fn sends_audio_during_capture_and_drains_tail_before_finish() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (first_frame, first_frame_seen) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws = accept_hdr_async(
            socket,
            |request: &tokio_tungstenite::tungstenite::handshake::server::Request, response| {
                assert_eq!(request.uri().path(), "/v1/dictate/stream");
                assert_eq!(
                    request.headers()["authorization"],
                    "Bearer sample-device-token"
                );
                Ok(response)
            },
        )
        .await
        .unwrap();
        let metadata = ws.next().await.unwrap().unwrap();
        assert_eq!(
            metadata,
            Message::Text(r#"{"cleanup_enabled":false}"#.into())
        );
        let frame = ws.next().await.unwrap().unwrap().into_data();
        assert_eq!(frame.len(), 1_440);
        ws.send(Message::Text(r#"{"type":"partial","text":"draft"}"#.into()))
            .await
            .unwrap();
        first_frame.send(()).unwrap();

        let mut saw_tail = false;
        loop {
            match ws.next().await.unwrap().unwrap() {
                Message::Binary(frame) => {
                    assert_eq!(frame.len(), 1_440);
                    saw_tail |= frame
                        .chunks_exact(2)
                        .any(|value| i16::from_le_bytes([value[0], value[1]]) > 15_000);
                }
                Message::Text(message) => {
                    assert!(
                        saw_tail,
                        "stop committed before the resampler tail was sent"
                    );
                    assert_eq!(message, r#"{"type":"finish"}"#);
                    break;
                }
                message => panic!("unexpected message: {message:?}"),
            }
        }
        ws.send(Message::Text(r#"{"raw_text":"最终稿","formatted_text":"最终稿","cleanup_applied":false,"fallback_reason":null,"asr_ms":10,"cleanup_ms":0}"#.into())).await.unwrap();
    });
    let (send_audio, audio) = mpsc::channel(8);
    let client = tokio::spawn(async move {
        exchange(
            &format!("http://{address}"),
            HeaderValue::from_static("Bearer sample-device-token"),
            r#"{"cleanup_enabled":false}"#.into(),
            audio,
        )
        .await
    });

    send_audio
        .send(AudioCommand::Audio(vec![0.0; 2_048]))
        .await
        .unwrap();
    timeout(WAIT, first_frame_seen)
        .await
        .expect("audio should reach gateway before key release")
        .unwrap();
    send_audio
        .send(AudioCommand::Audio(vec![0.75; 320]))
        .await
        .unwrap();
    send_audio.send(AudioCommand::Finish).await.unwrap();
    let reply = timeout(WAIT, client).await.unwrap().unwrap().unwrap();
    assert_eq!(reply.raw_text, "最终稿");
    timeout(WAIT, server).await.unwrap().unwrap();
}

#[tokio::test]
async fn drops_premature_final_instead_of_delivering_it() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(socket).await.unwrap();
        ws.next().await.unwrap().unwrap();
        ws.send(Message::Text(r#"{"raw_text":"premature","formatted_text":"premature","cleanup_applied":false,"asr_ms":0,"cleanup_ms":0}"#.into())).await.unwrap();
    });
    let (_send_audio, audio) = mpsc::channel(1);
    let result = timeout(
        WAIT,
        exchange(
            &format!("http://{address}"),
            HeaderValue::from_static("Bearer sample-device-token"),
            "{}".into(),
            audio,
        ),
    )
    .await
    .unwrap();
    assert_eq!(result.err(), Some("gateway_stream_unexpected_final"));
    timeout(WAIT, server).await.unwrap().unwrap();
}

#[tokio::test]
async fn overflowing_audio_queue_prevents_a_truncated_commit() {
    let route = AudioRoute::default();
    let id = Arc::new(());
    let (sender, mut receiver) = mpsc::channel(1);
    route.attach(id.clone(), sender);
    route.feed(&[0.2]);
    route.feed(&[0.3]);

    assert!(route.take(&id).is_none());
    let Some(AudioCommand::Audio(samples)) = receiver.recv().await else {
        panic!("the first queued frame should still be readable");
    };
    assert_eq!(samples, [0.2]);
    assert!(receiver.recv().await.is_none());
}

#[tokio::test]
async fn dropping_recording_closes_socket_without_sending_finish() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (received_audio, audio_seen) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(socket).await.unwrap();
        assert!(ws.next().await.unwrap().unwrap().is_text());
        assert!(ws.next().await.unwrap().unwrap().is_binary());
        received_audio.send(()).unwrap();
        while let Some(Ok(message)) = ws.next().await {
            assert!(!message.is_text(), "cancellation should not commit audio");
        }
    });
    let route = Arc::new(AudioRoute::default());
    let recording = Recording::start(
        route.clone(),
        format!("http://{address}"),
        HeaderValue::from_static("Bearer sample-device-token"),
        || Ok("{}".into()),
    );
    route.feed(&[0.3; 2_048]);
    timeout(WAIT, audio_seen).await.unwrap().unwrap();
    drop(recording);
    timeout(WAIT, server).await.unwrap().unwrap();
}

#[tokio::test]
async fn silent_gateway_is_disconnected_at_live_finalization_deadline() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (received_audio, audio_seen) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(socket).await.unwrap();
        ws.next().await.unwrap().unwrap();
        assert!(ws.next().await.unwrap().unwrap().is_binary());
        received_audio.send(()).unwrap();
        loop {
            match ws.next().await.unwrap().unwrap() {
                Message::Text(text) => {
                    assert_eq!(text, r#"{"type":"finish"}"#);
                    break;
                }
                Message::Binary(_) => {}
                message => panic!("unexpected message: {message:?}"),
            }
        }
        // Simulate a gateway that keeps the socket open without replying.
        assert!(ws.next().await.is_none_or(
            |message| message.is_err() || message.is_ok_and(|message| message.is_close())
        ));
    });
    let route = Arc::new(AudioRoute::default());
    let recording = Recording::start(
        route.clone(),
        format!("http://{address}"),
        HeaderValue::from_static("Bearer sample-device-token"),
        || Ok("{}".into()),
    );
    route.feed(&[0.3; 2_048]);
    timeout(WAIT, audio_seen).await.unwrap().unwrap();
    let result = recording.finish(Duration::from_millis(500)).await;
    assert_eq!(result.err(), Some("gateway_stream_timeout"));
    timeout(WAIT, server).await.unwrap().unwrap();
}
