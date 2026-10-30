use crate::audio_feedback::{play_feedback_sound, play_feedback_sound_blocking, SoundType};
use crate::audio_toolkit::{is_microphone_access_denied, is_no_input_device_error, VadPolicy};
use crate::dictation_cleanup::{self, CleanupOutcome, CleanupSession};
use crate::managers::audio::AudioRecordingManager;
use crate::managers::history::HistoryManager;
use crate::managers::model::ModelManager;
use crate::managers::transcription::StreamWorkKind;
use crate::managers::transcription::TranscriptionManager;
use crate::settings::{get_settings, OverlayStyle};
use crate::shortcut;
use crate::tray::{set_tray_state, TrayIconState};
use crate::utils::{
    self, show_processing_overlay, show_recording_overlay, show_transcribing_overlay,
};
use crate::TranscriptionCoordinator;
use log::{debug, error, warn};
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::Manager;
use tauri::{AppHandle, Emitter};

const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(25);

#[path = "paste_delivery.rs"]
pub(crate) mod paste_delivery;

#[derive(Clone, serde::Serialize)]
struct RecordingErrorEvent {
    error_type: String,
    detail: Option<String>,
}

#[derive(Clone, serde::Serialize)]
struct PasteErrorEvent {
    code: &'static str,
}

fn emit_paste_error(app: &AppHandle, error: &str) {
    let _ = app.emit(
        "paste-error",
        PasteErrorEvent {
            code: paste_delivery::paste_error_code(error),
        },
    );
}

/// Drop guard that finishes the transcription pipeline, including immediate
/// model unloading on early exits.
struct FinishGuard(AppHandle, Arc<TranscriptionManager>, Option<u64>);
impl Drop for FinishGuard {
    fn drop(&mut self) {
        // Esc remains registered throughout recognition and cleanup. Remove it
        // before notifying the coordinator, which may immediately start a
        // queued recording and register its own cancellation lifecycle.
        shortcut::unregister_cancel_shortcut(&self.0);
        self.1.maybe_unload_immediately("transcription session");
        if let (Some(c), Some(recording_id)) =
            (self.0.try_state::<TranscriptionCoordinator>(), self.2)
        {
            c.notify_processing_finished(recording_id);
        }
        // The pipeline just freed its large transient buffers (captured PCM,
        // WAV copy, engine scratch); hand the cached pages back to the OS so
        // they don't sit in malloc arenas until they get swapped out (#1792).
        crate::memory::trim_freed_memory();
    }
}

// Shortcut Action Trait
pub trait ShortcutAction: Send + Sync {
    fn start(&self, app: &AppHandle, binding_id: &str, shortcut_str: &str);
    fn stop(&self, app: &AppHandle, binding_id: &str, shortcut_str: &str);
}

// Both legacy shortcut names use the selected dictation cleanup mode.
struct TranscribeAction;

// None is a recording cancellation tombstone, consumed by a racing stop().
type TimedCleanupSession = (CleanupSession, Instant);
static CLEANUP_SESSIONS: Lazy<Mutex<HashMap<String, Option<TimedCleanupSession>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

async fn complete_unless_cancelled<F, C>(operation: F, is_cancelled: C) -> Option<F::Output>
where
    F: Future,
    C: Fn() -> bool,
{
    tokio::pin!(operation);

    loop {
        if is_cancelled() {
            return None;
        }

        if let Ok(result) =
            tokio::time::timeout(CANCELLATION_POLL_INTERVAL, operation.as_mut()).await
        {
            return Some(result);
        }
    }
}

fn should_use_streaming_overlay(style: OverlayStyle, is_streaming: bool) -> bool {
    style == OverlayStyle::Live && is_streaming
}

pub(crate) struct ProcessedTranscription {
    pub raw_text: String,
    pub final_text: String,
    pub post_processed_text: Option<String>,
    pub post_process_prompt: Option<String>,
    pub post_process_requested: bool,
    pub cleanup_ms: u64,
    pub cleanup_budget_ms: u64,
    pub cleanup_outcome: String,
}

fn processed_outcome(outcome: CleanupOutcome) -> ProcessedTranscription {
    // Persist both raw and final even when cleanup is off or falls back. No
    // provider payload, key, or user text enters a diagnostic/status field.
    let final_text = crate::dictation_lifecycle::normalize_output(&outcome.final_text);
    ProcessedTranscription {
        raw_text: outcome.raw_text,
        final_text: final_text.clone(),
        post_processed_text: Some(final_text),
        post_process_prompt: None,
        post_process_requested: outcome.requested,
        cleanup_ms: outcome.elapsed_ms,
        cleanup_budget_ms: outcome.budget_ms,
        cleanup_outcome: outcome
            .reason
            .unwrap_or(if outcome.requested { "cleaned" } else { "off" })
            .into(),
    }
}

pub(crate) async fn process_transcription_output(
    app: &AppHandle,
    transcription: &str,
    _legacy_post_process: bool,
) -> ProcessedTranscription {
    let prepared = crate::vocabulary::apply_confirmed(app, transcription);
    let mut processed =
        processed_outcome(dictation_cleanup::cleanup(app, &get_settings(app), &prepared).await);
    processed.raw_text = transcription.into();
    processed
}

async fn process_with_session(
    app: &AppHandle,
    transcription: &str,
    session: &CleanupSession,
) -> ProcessedTranscription {
    let prepared = crate::vocabulary::apply_confirmed(app, transcription);
    let mut processed =
        processed_outcome(dictation_cleanup::cleanup_with_session(app, session, &prepared).await);
    processed.raw_text = transcription.into();
    processed
}

impl ShortcutAction for TranscribeAction {
    fn start(&self, app: &AppHandle, binding_id: &str, _shortcut_str: &str) {
        let start_time = Instant::now();
        debug!("TranscribeAction::start called for binding: {}", binding_id);
        if let Ok(mut sessions) = CLEANUP_SESSIONS.lock() {
            sessions.remove(binding_id);
        }

        // Load model in the background
        let tm = app.state::<Arc<TranscriptionManager>>();
        let rm = app.state::<Arc<AudioRecordingManager>>();

        // Load ASR model and VAD model in parallel
        let kickoff_started = Instant::now();
        tm.initiate_model_load();
        let rm_clone = Arc::clone(&rm);
        std::thread::spawn(move || {
            if let Err(e) = rm_clone.preload_vad() {
                debug!("VAD pre-load failed: {}", e);
            }
        });
        let kickoff_elapsed = kickoff_started.elapsed();

        // Don't open the mic if nothing can transcribe the recording; the load
        // kicked off above fails and reports why.
        if !tm.is_model_loaded() {
            let selected_model = get_settings(app).selected_model;
            if let Err(e) = app
                .state::<Arc<ModelManager>>()
                .get_model_path(&selected_model)
            {
                warn!("Not starting recording: no model can transcribe it ({})", e);
                return;
            }
        }

        let binding_id = binding_id.to_string();
        let tray_started = Instant::now();
        set_tray_state(app, TrayIconState::Recording);
        let tray_elapsed = tray_started.elapsed();

        // Get the microphone mode to determine audio feedback timing
        let plan_started = Instant::now();
        let settings = get_settings(app);
        let is_always_on = settings.always_on_microphone;

        let selected_model_info = app
            .state::<Arc<ModelManager>>()
            .get_model_info(&settings.selected_model);

        // Use the app-facing model capability as the single pre-recording source
        // for live streaming decisions. Unknown support is represented as false
        // until the model registry is updated by discovery or runtime load.
        let model_supports_streaming = selected_model_info
            .as_ref()
            .map(|m| m.supports_streaming)
            .unwrap_or(false);
        let vad_policy = if !settings.vad_enabled {
            VadPolicy::Disabled
        } else if model_supports_streaming {
            VadPolicy::Streaming
        } else {
            VadPolicy::Offline
        };
        if model_supports_streaming {
            tm.start_stream();
        }
        let plan_elapsed = plan_started.elapsed();

        // Sizing the overlay follows the same advertised capability. A model that
        // doesn't stream (or whose capability is not known yet) gets the compact
        // pill instead of an oversized transparent live window.
        let overlay_started = Instant::now();
        match settings.overlay_style {
            OverlayStyle::Live if model_supports_streaming => utils::show_streaming_overlay(app),
            // Register the task even when its indicator is disabled. The
            // overlay renderer applies the visibility preference separately.
            OverlayStyle::Live | OverlayStyle::Minimal | OverlayStyle::None => {
                show_recording_overlay(app)
            }
        }
        // Everything above runs before capture can begin, so each span here is
        // added keypress->capture latency.
        debug!(
            "start-path pre-recording steps: model_kickoff={:?} tray={:?} settings+stream_plan={:?} overlay={:?}",
            kickoff_elapsed,
            tray_elapsed,
            plan_elapsed,
            overlay_started.elapsed()
        );
        debug!("Microphone mode - always_on: {}", is_always_on);

        // Freeze the upload route, model, timeout and credential before opening
        // the microphone; never re-read them in the async transcription path.
        let cleanup_session = dictation_cleanup::capture_session(app);
        if let Ok(mut sessions) = CLEANUP_SESSIONS.lock() {
            sessions.insert(binding_id.clone(), Some((cleanup_session, Instant::now())));
        }
        let mut recording_error: Option<String> = None;
        let recording_start_time = Instant::now();
        match rm.try_start_recording(&binding_id, vad_policy) {
            Ok(readiness) => {
                crate::vocabulary::clear_recent_corrections(app);
                debug!(
                    "Recording request accepted in {:?}; waiting for first microphone samples",
                    recording_start_time.elapsed()
                );
                let generation = readiness.generation();
                let app_clone = app.clone();
                let rm_clone = Arc::clone(&rm);
                std::thread::spawn(move || {
                    if !readiness.wait() {
                        debug!("Microphone readiness wait ended without receiving samples");
                        return;
                    }

                    // Development-only preview hook for evaluating the brief
                    // arming animation on hardware that normally starts too fast
                    // to make it visible.
                    #[cfg(debug_assertions)]
                    if let Ok(delay_ms) = std::env::var("HANDY_DEBUG_MIC_READY_DELAY_MS")
                        .unwrap_or_default()
                        .parse::<u64>()
                    {
                        let delay_ms = delay_ms.min(10_000);
                        if delay_ms > 0 {
                            debug!("Delaying microphone-ready cue by {delay_ms}ms for UI preview");
                            std::thread::sleep(Duration::from_millis(delay_ms));
                        }
                    }

                    if !rm_clone.is_recording_readiness_current(generation) {
                        debug!("Microphone became ready for an inactive recording");
                        return;
                    }

                    debug!("Microphone is receiving samples; recording is ready");
                    utils::emit_recording_ready(&app_clone);

                    // The start chime is a readiness cue, so it must follow the
                    // first real input callback rather than Stream::play() or a
                    // fixed delay. The helper returns immediately when feedback
                    // is disabled; mute still follows the same readiness point.
                    if rm_clone.is_recording_readiness_current(generation) {
                        play_feedback_sound_blocking(&app_clone, SoundType::Start);
                    }
                    if rm_clone.is_recording_readiness_current(generation) {
                        rm_clone.apply_mute();
                    }
                });
            }
            Err(e) => {
                debug!("Failed to start recording: {}", e);
                recording_error = Some(e);
            }
        }

        if recording_error.is_none() {
            // Dynamically register the cancel shortcut in a separate task to avoid deadlock
            shortcut::register_cancel_shortcut(app);
        } else {
            if let Ok(mut sessions) = CLEANUP_SESSIONS.lock() {
                sessions.remove(&binding_id);
            }
            // Starting failed (for example due to blocked microphone permissions).
            // Revert UI state so we don't stay stuck in the recording overlay.
            tm.cancel_stream();
            utils::hide_recording_overlay(app);
            set_tray_state(app, TrayIconState::Idle);
            if let Some(err) = recording_error {
                let error_type = if is_microphone_access_denied(&err) {
                    "microphone_permission_denied"
                } else if is_no_input_device_error(&err) {
                    "no_input_device"
                } else {
                    "unknown"
                };
                let _ = app.emit(
                    "recording-error",
                    RecordingErrorEvent {
                        error_type: error_type.to_string(),
                        detail: Some(err),
                    },
                );
            }
        }

        debug!(
            "TranscribeAction::start completed in {:?}",
            start_time.elapsed()
        );
    }

    fn stop(&self, app: &AppHandle, binding_id: &str, _shortcut_str: &str) {
        let recording_id = app
            .try_state::<TranscriptionCoordinator>()
            .and_then(|coordinator| coordinator.current_recording_id());
        // Capture before claiming the session: once it is claimed, Esc can
        // cancel from another callback before the processing task is spawned.
        let cancel_generation = app
            .state::<Arc<AudioRecordingManager>>()
            .cancel_generation();
        // Claim the recording before changing its UI. Cancellation drops the
        // credential-bearing session and leaves a tombstone, so a release that
        // races with Esc cannot produce a second cancellation/no-audio record.
        let (cleanup_session, recording_started_at) = match CLEANUP_SESSIONS
            .lock()
            .ok()
            .and_then(|mut sessions| sessions.remove(binding_id))
        {
            Some(Some((session, started))) => (session, Some(started)),
            Some(None) => {
                if let (Some(coordinator), Some(id)) =
                    (app.try_state::<TranscriptionCoordinator>(), recording_id)
                {
                    coordinator.notify_stop_aborted(id);
                }
                return;
            }
            None => (CleanupSession::unavailable(), None),
        };
        // Prevent a slow microphone from emitting a ready event or start chime
        // after the user has already requested stop.
        app.state::<Arc<AudioRecordingManager>>()
            .invalidate_recording_readiness();

        let stop_time = Instant::now();
        let recording_ms =
            crate::dictation_metrics::recording_elapsed_ms(recording_started_at, stop_time);
        debug!("TranscribeAction::stop called for binding: {}", binding_id);

        let ah = app.clone();
        let rm = Arc::clone(&app.state::<Arc<AudioRecordingManager>>());
        let tm = Arc::clone(&app.state::<Arc<TranscriptionManager>>());
        let hm = Arc::clone(&app.state::<Arc<HistoryManager>>());

        set_tray_state(app, TrayIconState::Transcribing);
        // Stop should give immediate visual feedback. Live streaming can keep
        // the larger panel, but it still switches from listening to a working
        // spinner while the stream finalizes. Non-streaming paths use the
        // compact transcribing pill (None no-ops in show_*).
        let style = get_settings(app).overlay_style;
        // Capture this before finalizing the stream so every later working state
        // targets the same overlay that was shown for this transcription.
        let use_streaming_overlay = should_use_streaming_overlay(style, tm.is_streaming());
        if use_streaming_overlay {
            tm.emit_stream_working(StreamWorkKind::Transcribing);
        } else {
            show_transcribing_overlay(app);
        }

        let binding_id = binding_id.to_string();
        let post_process = cleanup_session.is_enabled();
        let mut timing =
            crate::dictation_metrics::DictationTiming::new(cleanup_session.mode_name());
        timing.recording_ms = recording_ms;
        timing.begin_resources();

        tauri::async_runtime::spawn(async move {
            let guard = Arc::new(FinishGuard(ah.clone(), Arc::clone(&tm), recording_id));
            debug!(
                "Starting async transcription task for binding: {}",
                binding_id
            );

            let stop_recording_time = Instant::now();
            let samples = rm.stop_recording(&binding_id, cancel_generation);
            // stop_recording seals the final captured block and restores playback
            // before this sound. Neither playback nor our chime can enter the tail.
            if !rm.was_cancelled_since(cancel_generation) {
                play_feedback_sound(&ah, SoundType::Stop);
            }
            if let Some(samples) = samples {
                debug!(
                    "Recording stopped and samples retrieved in {:?}, sample count: {}",
                    stop_recording_time.elapsed(),
                    samples.len()
                );

                if rm.was_cancelled_since(cancel_generation) {
                    debug!("Transcription operation cancelled after recording stop");
                    timing.outcome = "cancelled".into();
                    timing.total_ms = crate::dictation_metrics::elapsed_ms(stop_time);
                    crate::dictation_metrics::record(&ah, timing);
                    tm.cancel_stream();
                    utils::hide_recording_overlay(&ah);
                    set_tray_state(&ah, TrayIconState::Idle);
                    return;
                }

                if samples.is_empty() {
                    debug!("Recording produced no audio samples; skipping persistence");
                    timing.outcome = "no_audio".into();
                    timing.total_ms = crate::dictation_metrics::elapsed_ms(stop_time);
                    crate::dictation_metrics::record(&ah, timing);
                    // Tear down any streaming worker so its channel doesn't leak
                    // and block the next start_stream.
                    tm.cancel_stream();
                    utils::hide_recording_overlay(&ah);
                    set_tray_state(&ah, TrayIconState::Idle);
                } else {
                    // Save WAV concurrently with transcription
                    let sample_count = samples.len();
                    let file_name = format!("handy-{}.wav", chrono::Utc::now().timestamp());
                    let wav_path = hm.recordings_dir().join(&file_name);
                    let wav_path_for_verify = wav_path.clone();
                    let samples_for_wav = samples.clone();
                    let wav_handle = tauri::async_runtime::spawn_blocking(move || {
                        crate::audio_toolkit::save_wav_file(&wav_path, &samples_for_wav)
                    });

                    // Transcribe concurrently with WAV save. If a live stream was
                    // running, finalize it and use its text (all audio was already
                    // fed to the stream); otherwise batch-transcribe the samples.
                    let transcription_time = Instant::now();
                    let transcription_result = match tm.finalize_stream() {
                        // A finalized stream with usable text wins. An empty result
                        // (no active stream, produced nothing, or a finalize error
                        // after the engine was returned) falls back to a full batch
                        // transcription of the same audio. A finalize timeout is
                        // surfaced instead — the worker may still hold the engine,
                        // so a batch fallback would contend with it.
                        Ok(Some(text)) if !text.trim().is_empty() => Ok(text),
                        Ok(_) => tm.transcribe(samples),
                        Err(err) => Err(err),
                    };
                    timing.asr_ms = crate::dictation_metrics::elapsed_ms(transcription_time);
                    timing.snapshot_resources("asr");

                    // Await WAV save and verify
                    let wav_saved = match wav_handle.await {
                        Ok(Ok(())) => {
                            match crate::audio_toolkit::verify_wav_file(
                                &wav_path_for_verify,
                                sample_count,
                            ) {
                                Ok(()) => true,
                                Err(e) => {
                                    error!("WAV verification failed: {}", e);
                                    false
                                }
                            }
                        }
                        Ok(Err(e)) => {
                            error!("Failed to save WAV file: {}", e);
                            false
                        }
                        Err(e) => {
                            error!("WAV save task panicked: {}", e);
                            false
                        }
                    };

                    if rm.was_cancelled_since(cancel_generation) {
                        debug!("Transcription operation cancelled before output handling");
                        timing.outcome = "cancelled".into();
                        timing.total_ms = crate::dictation_metrics::elapsed_ms(stop_time);
                        crate::dictation_metrics::record(&ah, timing);
                        utils::hide_recording_overlay(&ah);
                        set_tray_state(&ah, TrayIconState::Idle);
                        return;
                    }

                    match transcription_result {
                        Ok(transcription) => {
                            debug!(
                                "Transcription completed in {:?}: '{}'",
                                transcription_time.elapsed(),
                                utils::redact_text(&transcription)
                            );

                            if post_process && !transcription.trim().is_empty() {
                                if use_streaming_overlay {
                                    tm.emit_stream_working(StreamWorkKind::Polishing);
                                } else {
                                    show_processing_overlay(&ah);
                                }
                            }
                            let cleanup_time = Instant::now();
                            let Some(processed) = complete_unless_cancelled(
                                process_with_session(&ah, &transcription, &cleanup_session),
                                || rm.was_cancelled_since(cancel_generation),
                            )
                            .await
                            else {
                                debug!("Transcription operation cancelled during output handling");
                                timing.cleanup_ms =
                                    crate::dictation_metrics::elapsed_ms(cleanup_time);
                                timing.outcome = "cancelled".into();
                                timing.total_ms = crate::dictation_metrics::elapsed_ms(stop_time);
                                crate::dictation_metrics::record(&ah, timing);
                                utils::hide_recording_overlay(&ah);
                                set_tray_state(&ah, TrayIconState::Idle);
                                return;
                            };
                            timing.snapshot_resources("cleanup");

                            if rm.was_cancelled_since(cancel_generation) {
                                debug!("Transcription operation cancelled before paste");
                                timing.cleanup_ms = processed.cleanup_ms;
                                timing.cleanup_budget_ms = processed.cleanup_budget_ms;
                                timing.cleanup_reason = (!matches!(
                                    processed.cleanup_outcome.as_str(),
                                    "cleaned" | "off"
                                ))
                                .then(|| processed.cleanup_outcome.clone());
                                timing.outcome = "cancelled".into();
                                timing.total_ms = crate::dictation_metrics::elapsed_ms(stop_time);
                                crate::dictation_metrics::record(&ah, timing);
                                utils::hide_recording_overlay(&ah);
                                set_tray_state(&ah, TrayIconState::Idle);
                                return;
                            }

                            // Recoverable text does not depend on successful
                            // audio persistence. Missing audio only disables
                            // playback/retranscription of this history entry.
                            timing.cleanup_ms = processed.cleanup_ms;
                            timing.cleanup_budget_ms = processed.cleanup_budget_ms;
                            timing.outcome = processed.cleanup_outcome.clone();
                            timing.cleanup_reason =
                                (!matches!(processed.cleanup_outcome.as_str(), "cleaned" | "off"))
                                    .then(|| processed.cleanup_outcome.clone());
                            match hm.save_entry(
                                file_name,
                                processed.raw_text,
                                processed.post_process_requested,
                                processed.post_processed_text.clone(),
                                processed.post_process_prompt.clone(),
                            ) {
                                Ok(entry) => {
                                    timing.history_id = Some(entry.id);
                                    let learning_app = ah.clone();
                                    tauri::async_runtime::spawn_blocking(move || {
                                        if crate::vocabulary::observe_transcript(
                                            &learning_app,
                                            &entry,
                                        )
                                        .is_err()
                                        {
                                            log::warn!("Could not update vocabulary candidates");
                                        }
                                    });
                                }
                                Err(err) => error!("Failed to save history entry: {}", err),
                            }

                            if processed.final_text.is_empty() {
                                timing.outcome = "empty".into();
                                timing.total_ms = crate::dictation_metrics::elapsed_ms(stop_time);
                                crate::dictation_metrics::record(&ah, timing);
                                utils::hide_recording_overlay(&ah);
                                set_tray_state(&ah, TrayIconState::Idle);
                            } else {
                                let ah_clone = ah.clone();
                                let paste_time = Instant::now();
                                let final_text = processed.final_text;
                                let rm_for_paste = Arc::clone(&rm);
                                let mut dispatch_failure_timing = timing.clone();
                                paste_delivery::dispatch_automatic_paste(Arc::clone(&guard), |work| ah.run_on_main_thread(work).map_err(|error| error.to_string()), move || {
                                    if rm_for_paste.was_cancelled_since(cancel_generation) {
                                        timing.outcome = "cancelled".into();
                                        timing.total_ms = crate::dictation_metrics::elapsed_ms(stop_time);
                                        crate::dictation_metrics::record(&ah_clone, timing);
                                        debug!("Transcription operation cancelled before paste");
                                        utils::hide_recording_overlay(&ah_clone);
                                        set_tray_state(&ah_clone, TrayIconState::Idle);
                                        return;
                                    }

                                    match utils::paste_cancellable(final_text, ah_clone.clone(), || rm_for_paste.was_cancelled_since(cancel_generation)) {
                                        Ok(paste_delivery::Submission::Cancelled) => {
                                            timing.outcome = "cancelled".into();
                                            debug!("Transcription operation cancelled before key injection");
                                        }
                                        Ok(evidence) => {
                                            match evidence {
                                                paste_delivery::Submission::ClipboardReadObserved => timing.outcome = "paste_read_observed".into(),
                                                paste_delivery::Submission::UnicodeSubmitted => timing.outcome = "unicode_input_submitted".into(),
                                                _ => {},
                                            }
                                            debug!("Input evidence {:?} in {:?}; target insertion is unconfirmed", evidence, paste_time.elapsed());
                                            timing.paste_ms = Some(crate::dictation_metrics::elapsed_ms(paste_time));
                                        }
                                        Err(e) if e == "rdp_input_submitted_copy_failed" => {
                                            timing.outcome = "unicode_input_submitted_copy_failed".into();
                                            timing.paste_ms = Some(crate::dictation_metrics::elapsed_ms(paste_time));
                                            warn!("Unicode input submitted; optional clipboard copy failed; not retrying");
                                            emit_paste_error(&ah_clone, &e);
                                        }
                                        Err(e) if e == "automatic_paste_unconfirmed" || e == "rdp_input_uncertain" => {
                                            timing.outcome = "paste_unconfirmed".into();
                                            warn!("Automatic input is unconfirmed; not retrying");
                                            emit_paste_error(&ah_clone, &e);
                                        }
                                        Err(e) if e == "automatic_paste_disabled" => {
                                            timing.outcome = "paste_skipped".into();
                                        }
                                        Err(e) => {
                                            timing.outcome = "paste_failed".into();
                                            error!("Failed to paste transcription: {}", e);
                                            emit_paste_error(&ah_clone, &e);
                                        }
                                    }
                                    timing.total_ms = crate::dictation_metrics::elapsed_ms(stop_time);
                                    crate::dictation_metrics::record(&ah_clone, timing);
                                    utils::hide_recording_overlay(&ah_clone);
                                    set_tray_state(&ah_clone, TrayIconState::Idle);
                                })
                                .unwrap_or_else(|e| {
                                    dispatch_failure_timing.outcome = "paste_failed".into();
                                    dispatch_failure_timing.total_ms = crate::dictation_metrics::elapsed_ms(stop_time);
                                    crate::dictation_metrics::record(&ah, dispatch_failure_timing);
                                    error!("Failed to dispatch automatic paste: {:?}", e);
                                    emit_paste_error(&ah, &e);
                                    utils::hide_recording_overlay(&ah);
                                    set_tray_state(&ah, TrayIconState::Idle);
                                });
                            }
                        }
                        Err(err) => {
                            timing.outcome = if rm.was_cancelled_since(cancel_generation) {
                                "cancelled"
                            } else {
                                "asr_failed"
                            }
                            .into();
                            timing.total_ms = crate::dictation_metrics::elapsed_ms(stop_time);
                            crate::dictation_metrics::record(&ah, timing);
                            if rm.was_cancelled_since(cancel_generation) {
                                debug!(
                                    "Transcription operation cancelled after transcription error"
                                );
                                utils::hide_recording_overlay(&ah);
                                set_tray_state(&ah, TrayIconState::Idle);
                                return;
                            }

                            error!("Transcription failed: {}", err);
                            // Surface the failure to the UI (toast). The full
                            // message is also in handy.log via the line above.
                            let _ = ah.emit("transcription-error", err.to_string());
                            // Save entry with empty text so user can retry
                            if wav_saved {
                                if let Err(save_err) = hm.save_entry(
                                    file_name,
                                    String::new(),
                                    post_process,
                                    None,
                                    None,
                                ) {
                                    error!("Failed to save failed history entry: {}", save_err);
                                }
                            }
                            utils::hide_recording_overlay(&ah);
                            set_tray_state(&ah, TrayIconState::Idle);
                        }
                    }
                }
            } else {
                debug!("No samples retrieved from recording stop");
                timing.outcome = crate::dictation_metrics::stopped_without_audio_outcome(
                    rm.was_cancelled_since(cancel_generation),
                )
                .into();
                timing.total_ms = crate::dictation_metrics::elapsed_ms(stop_time);
                crate::dictation_metrics::record(&ah, timing);
                // Tear down any streaming worker so its channel doesn't leak.
                tm.cancel_stream();
                utils::hide_recording_overlay(&ah);
                set_tray_state(&ah, TrayIconState::Idle);
            }
        });

        debug!(
            "TranscribeAction::stop completed in {:?}",
            stop_time.elapsed()
        );
    }
}

// Cancel Action
/// Discard unclaimed credentials on any cancellation, keeping tombstones so a
/// racing release cannot turn a cancelled recording into a fresh operation.
pub fn discard_pending_cleanup_sessions() {
    if let Ok(mut sessions) = CLEANUP_SESSIONS.lock() {
        for session in sessions.values_mut() {
            *session = None;
        }
    }
}

struct CancelAction;

impl ShortcutAction for CancelAction {
    fn start(&self, app: &AppHandle, _binding_id: &str, _shortcut_str: &str) {
        // stop() takes the session before beginning processing. Only a session
        // still owned by an active recording needs its own cancellation row;
        // the processing task records its cancellation otherwise.
        let recording_modes = CLEANUP_SESSIONS
            .lock()
            .map(|mut sessions| {
                let active = app.state::<Arc<AudioRecordingManager>>().is_recording();
                let modes: Vec<_> = sessions
                    .values_mut()
                    .filter_map(|session| {
                        crate::dictation_metrics::claim_recording_cancel(active, session)
                            .map(|(session, _)| session.mode_name())
                    })
                    .collect();
                if !active {
                    // Keep cancellation tombstones until stop/new start consumes
                    // them. A repeated Esc must not revive a racing stop.
                    for session in sessions.values_mut() {
                        *session = None;
                    }
                }
                modes
            })
            .unwrap_or_default();
        utils::cancel_current_operation(app);
        for mode in recording_modes {
            let mut timing = crate::dictation_metrics::DictationTiming::new(mode);
            timing.outcome = "cancelled".into();
            crate::dictation_metrics::record(app, timing);
        }
    }

    fn stop(&self, _app: &AppHandle, _binding_id: &str, _shortcut_str: &str) {
        // Nothing to do on stop for cancel
    }
}

// Test Action
struct TestAction;

impl ShortcutAction for TestAction {
    fn start(&self, app: &AppHandle, binding_id: &str, shortcut_str: &str) {
        log::info!(
            "Shortcut ID '{}': Started - {} (App: {})", // Changed "Pressed" to "Started" for consistency
            binding_id,
            shortcut_str,
            app.package_info().name
        );
    }

    fn stop(&self, app: &AppHandle, binding_id: &str, shortcut_str: &str) {
        log::info!(
            "Shortcut ID '{}': Stopped - {} (App: {})", // Changed "Released" to "Stopped" for consistency
            binding_id,
            shortcut_str,
            app.package_info().name
        );
    }
}

// Static Action Map
pub static ACTION_MAP: Lazy<HashMap<String, Arc<dyn ShortcutAction>>> = Lazy::new(|| {
    let mut map = HashMap::new();
    map.insert(
        "transcribe".to_string(),
        Arc::new(TranscribeAction) as Arc<dyn ShortcutAction>,
    );
    map.insert(
        "transcribe_with_post_process".to_string(),
        Arc::new(TranscribeAction) as Arc<dyn ShortcutAction>,
    );
    map.insert(
        "cancel".to_string(),
        Arc::new(CancelAction) as Arc<dyn ShortcutAction>,
    );
    map.insert(
        "test".to_string(),
        Arc::new(TestAction) as Arc<dyn ShortcutAction>,
    );
    map
});

/// Share the shortcut cancellation lifecycle with explicit user UI/CLI actions.
pub fn cancel_user_operation(app: &AppHandle) {
    if let Some(action) = ACTION_MAP.get("cancel") {
        action.start(app, "cancel", "");
    } else {
        utils::cancel_current_operation(app);
    }
}

#[cfg(test)]
mod tests {
    use super::{complete_unless_cancelled, should_use_streaming_overlay};
    use crate::settings::OverlayStyle;
    use std::future;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn completed_operation_returns_its_output() {
        let result = tauri::async_runtime::block_on(complete_unless_cancelled(
            future::ready("done"),
            || false,
        ));

        assert_eq!(result, Some("done"));
    }

    #[test]
    fn pending_operation_stops_after_cancellation() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancelled_for_thread = Arc::clone(&cancelled);
        let cancel_thread = thread::spawn(move || {
            thread::sleep(Duration::from_millis(10));
            cancelled_for_thread.store(true, Ordering::Release);
        });

        let result = tauri::async_runtime::block_on(complete_unless_cancelled(
            future::pending::<()>(),
            || cancelled.load(Ordering::Acquire),
        ));

        cancel_thread.join().unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn live_overlay_uses_streaming_states_only_for_streaming_models() {
        assert!(should_use_streaming_overlay(OverlayStyle::Live, true));
        assert!(!should_use_streaming_overlay(OverlayStyle::Live, false));
        assert!(!should_use_streaming_overlay(OverlayStyle::Minimal, true));
        assert!(!should_use_streaming_overlay(OverlayStyle::None, true));
    }
}
