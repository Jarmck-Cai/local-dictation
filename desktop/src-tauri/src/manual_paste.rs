//! One explicit output attempt, using only the existing completed-history body.
//! This module never starts transcription, cloud cleanup, or a retry.
use crate::managers::audio::AudioRecordingManager;
use crate::managers::history::HistoryManager;
use crate::settings::{self, ShortcutBinding};
use crate::transcription_coordinator::ManualPasteRejection;
use crate::TranscriptionCoordinator;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

#[path = "shortcut/binding_conflicts.rs"]
mod binding_conflicts;
#[path = "manual_paste_keyboard.rs"]
mod keyboard;
#[path = "manual_paste_policy.rs"]
mod policy;
use policy::{GestureLatch, ManualOutcome, ReleaseDecision, ReleaseWait};

static MANUAL_PRESS: GestureLatch = GestureLatch::new();
const RELEASE_TIMEOUT: Duration = Duration::from_secs(2);
const RELEASE_POLL: Duration = Duration::from_millis(10);

/// At most one release waiter for a held gesture, including recovery after
/// its first deadline. Old waiters cannot clear a newer gesture's watcher.
struct ReleaseWatch(u64);
impl ReleaseWatch {
    fn begin(ticket: u64) -> Option<Self> {
        MANUAL_PRESS
            .begin_release_watch(ticket)
            .then(|| Self(ticket))
    }
    fn consume(self, started: Instant) {
        consume_until_release(started, self.0);
    }
}
impl Drop for ReleaseWatch {
    fn drop(&mut self) {
        MANUAL_PRESS.finish_release_watch(self.0);
    }
}

#[derive(Clone, serde::Serialize)]
struct ManualPasteEvent {
    outcome: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_code: Option<String>,
}

fn emit(app: &AppHandle, outcome: &'static str, error_code: Option<String>) {
    // Codes only: history contents and OS/clipboard error prose never enter logs.
    log::debug!("Manual paste outcome: {outcome}");
    let _ = app.emit(
        "dictation-manual-paste",
        ManualPasteEvent {
            outcome,
            error_code,
        },
    );
}

fn emit_result(app: &AppHandle, outcome: ManualOutcome) {
    let (outcome, error_code) = match outcome {
        ManualOutcome::ReadObserved => ("read_observed", None),
        ManualOutcome::InputSent => ("input_sent", None),
        ManualOutcome::NoResult => ("no_result", None),
        ManualOutcome::Cancelled => ("cancelled", None),
        ManualOutcome::KeysHeld => ("keys_held", None),
        ManualOutcome::Unsupported => ("unsupported", None),
        ManualOutcome::KeyboardUnavailable => ("keyboard_unavailable", None),
        ManualOutcome::HistoryFailed => ("history_failed", None),
        ManualOutcome::Failed(code) => {
            let code = if !code.is_empty()
                && code.len() <= 80
                && code
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
            {
                code
            } else {
                "paste_failed".into()
            };
            ("failed", Some(code))
        }
    };
    emit(app, outcome, error_code);
}

fn rejection(app: &AppHandle, reason: ManualPasteRejection) {
    emit(
        app,
        match reason {
            ManualPasteRejection::Busy => "busy",
            ManualPasteRejection::Unavailable => "unavailable",
        },
        None,
    );
}

/// Invoked by both shortcut backends. The press latch consumes a busy gesture
/// too, so auto-repeat cannot turn it into a request after processing drains.
pub fn handle_shortcut(app: &AppHandle, is_pressed: bool) {
    if !is_pressed {
        // A backend's combo-release can mean only a modifier came up. Keep
        // the gesture consumed until the aggregate physical keyboard is up.
        if let Some(ticket) = MANUAL_PRESS.current() {
            if matches!(any_keyboard_key_down(), Ok(false)) {
                MANUAL_PRESS.release(ticket);
            }
        }
        return;
    }
    let Some(ticket) = MANUAL_PRESS.claim() else {
        if let Some(watch) = MANUAL_PRESS.current().and_then(ReleaseWatch::begin) {
            // Recovery only: even after a prior deadline, this edge cannot
            // become a paste. A full release re-arms the next deliberate press.
            emit(app, "release_to_retry", None);
            consume_rejected_gesture(Instant::now(), watch);
        }
        return;
    };
    let Some(watch) = ReleaseWatch::begin(ticket) else {
        return;
    };
    let started = Instant::now();
    let Some(audio) = app.try_state::<Arc<AudioRecordingManager>>() else {
        emit(app, "unavailable", None);
        consume_rejected_gesture(started, watch);
        return;
    };
    let audio = Arc::clone(&audio);
    let cancel_generation = audio.cancel_generation();
    let Some(coordinator) = app.try_state::<TranscriptionCoordinator>() else {
        emit(app, "unavailable", None);
        consume_rejected_gesture(started, watch);
        return;
    };
    // Enqueue synchronously on the key-down, before scheduling the worker.
    let request = match coordinator.request_manual_paste() {
        Ok(request) => request,
        Err(reason) => {
            rejection(app, reason);
            consume_rejected_gesture(started, watch);
            return;
        }
    };
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let permit = match request.wait() {
            Ok(permit) => permit,
            Err(reason) => {
                rejection(&app, reason);
                watch.consume(started);
                return;
            }
        };
        let is_cancelled = || permit.is_cancelled() || audio.was_cancelled_since(cancel_generation);
        let result = policy::execute(
            is_cancelled,
            || {
                wait_for_release(started, is_cancelled)?;
                MANUAL_PRESS.release(ticket);
                Ok(())
            },
            || {
                app.try_state::<Arc<HistoryManager>>()
                    .ok_or(())?
                    .get_latest_result_body()
                    .map_err(|_| ())
            },
            |body| {
                // Recheck physical keys after the database read. No key state is
                // retained, and the protected transaction rechecks cancellation
                // immediately before its only injection attempt.
                if is_cancelled() {
                    return Err("cancelled".into());
                }
                match any_keyboard_key_down() {
                    Ok(false) => crate::clipboard::paste_latest_preserving_clipboard(
                        body,
                        app.clone(),
                        is_cancelled,
                    ),
                    Ok(true) => Err("keys_held".into()),
                    Err(code) => Err(code.into()),
                }
            },
        );
        // The permit remains alive until the protected call has restored the
        // clipboard, including cancellation and failure paths.
        drop(permit);
        emit_result(&app, result);
        // A cancellation can finish the serial operation before the gesture
        // has physically ended. Only this ticket may be released afterward.
        watch.consume(started);
    });
}

fn consume_rejected_gesture(started: Instant, watch: ReleaseWatch) {
    tauri::async_runtime::spawn_blocking(move || watch.consume(started));
}

fn consume_until_release(started: Instant, ticket: u64) {
    if MANUAL_PRESS.current() == Some(ticket) && wait_for_release(started, || false).is_ok() {
        MANUAL_PRESS.release(ticket);
    }
}

fn wait_for_release(
    started: Instant,
    is_cancelled: impl Fn() -> bool,
) -> Result<(), ManualOutcome> {
    let wait = ReleaseWait::new(started, RELEASE_TIMEOUT);
    loop {
        let cancelled = is_cancelled();
        if cancelled {
            return Err(ManualOutcome::Cancelled);
        }
        let any_key_down = any_keyboard_key_down().map_err(|code| match code {
            "unsupported" => ManualOutcome::Unsupported,
            _ => ManualOutcome::KeyboardUnavailable,
        })?;
        match wait.poll(Instant::now(), any_key_down, cancelled) {
            ReleaseDecision::Ready => return Ok(()),
            ReleaseDecision::Cancelled => return Err(ManualOutcome::Cancelled),
            ReleaseDecision::TimedOut => return Err(ManualOutcome::KeysHeld),
            ReleaseDecision::Pending => std::thread::sleep(RELEASE_POLL),
        }
    }
}

/// An on-demand aggregate query only. It excludes mouse VKs and never records
/// which key was held. It covers custom keys and left/right modifiers equally.
#[cfg(target_os = "windows")]
pub(crate) fn any_keyboard_key_down() -> Result<bool, &'static str> {
    #[link(name = "user32")]
    extern "system" {
        fn GetAsyncKeyState(key: i32) -> i16;
        fn GetForegroundWindow() -> *mut std::ffi::c_void;
    }
    // A missing foreground window is not evidence that all keys were released
    // (for example, while changing secure desktops); fail before touching data.
    if unsafe { GetForegroundWindow() }.is_null() {
        return Err("keyboard_unavailable");
    }
    Ok((0x08..=0xfe).any(|key| unsafe { GetAsyncKeyState(key) } < 0))
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn any_keyboard_key_down() -> Result<bool, &'static str> {
    Err("unsupported")
}

/// Shared protected-paste chord, invoked inside with_enigo's locked closure
/// immediately before its only key attempt.
pub(crate) fn send_paste_chord(
    enigo: &mut enigo::Enigo,
    method: settings::PasteMethod,
    is_cancelled: impl Fn() -> bool,
) -> Result<(), String> {
    use enigo::Keyboard;
    let chord = match method {
        settings::PasteMethod::CtrlV => keyboard::Chord::CtrlV,
        settings::PasteMethod::CtrlShiftV => keyboard::Chord::CtrlShiftV,
        settings::PasteMethod::ShiftInsert => keyboard::Chord::ShiftInsert,
        _ => return Err("manual_paste_unsupported_method".into()),
    };
    struct Adapter<'a>(&'a mut enigo::Enigo);
    impl keyboard::Keyboard for Adapter<'_> {
        fn key(&mut self, key: keyboard::Key, direction: keyboard::Direction) -> Result<(), ()> {
            let key = match key {
                keyboard::Key::Control => enigo::Key::Control,
                keyboard::Key::Shift => enigo::Key::Shift,
                keyboard::Key::V => enigo::Key::Other(0x56),
                keyboard::Key::Insert => enigo::Key::Other(0x2d),
            };
            let direction = match direction {
                keyboard::Direction::Press => enigo::Direction::Press,
                keyboard::Direction::Release => enigo::Direction::Release,
            };
            self.0.key(key, direction).map_err(|_| ())
        }
    }
    keyboard::inject_if_released(
        &mut Adapter(enigo),
        chord,
        any_keyboard_key_down,
        is_cancelled,
        || std::thread::sleep(Duration::from_millis(100)),
    )
    .map_err(String::from)
}

pub(crate) fn conflicts(settings: &settings::AppSettings, candidate: &str) -> bool {
    binding_conflicts::conflicts(
        candidate,
        settings
            .bindings
            .iter()
            .map(|(id, binding)| (id.as_str(), binding.current_binding.as_str())),
    )
}

/// Shared by both registration backends, initialization, switching and resume.
pub fn binding_is_conflicted(app: &AppHandle, binding: &ShortcutBinding) -> bool {
    binding.id == "paste_latest_result"
        && conflicts(&settings::get_settings(app), &binding.current_binding)
}

pub fn ensure_binding_available(app: &AppHandle, binding: &ShortcutBinding) -> Result<(), String> {
    if binding_is_conflicted(app, binding) {
        emit(app, "binding_conflict", None);
        return Err("manual_paste_binding_conflict".into());
    }
    Ok(())
}

/// Lets the settings UI show a persistent warning without duplicating parsing.
#[tauri::command]
#[specta::specta]
pub fn get_manual_paste_binding_conflict(app: AppHandle) -> bool {
    let settings = settings::get_settings(&app);
    settings
        .bindings
        .get("paste_latest_result")
        .is_some_and(|binding| conflicts(&settings, &binding.current_binding))
}
