//! Application boundary for the local meeting actors.
use super::{
    contracts::{MeetingSummary, MeetingTranscript},
    runtime::{MeetingRuntime, NativeCaptureFactory, RuntimeServices},
    store::MeetingStore,
    types::DeviceInfo,
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tauri::{AppHandle, Emitter, Manager};

#[cfg(all(target_os = "windows", target_arch = "x86_64", target_env = "msvc"))]
struct MeetingModelUse(Arc<crate::managers::transcription::TranscriptionManager>);

#[cfg(all(target_os = "windows", target_arch = "x86_64", target_env = "msvc"))]
impl Drop for MeetingModelUse {
    fn drop(&mut self) {
        self.0.finish_meeting_model_use();
    }
}

fn supported() -> bool {
    cfg!(all(
        target_os = "windows",
        target_arch = "x86_64",
        target_env = "msvc"
    ))
}

fn transcription_error(error: anyhow::Error) -> String {
    if error.root_cause().to_string() == "meeting_model_unsupported" {
        "meeting_model_unsupported".into()
    } else {
        "meeting_transcription_failed".into()
    }
}

pub fn initialize(app: &AppHandle) -> Result<(), String> {
    if !supported() {
        return Ok(());
    }
    let root = crate::portable::app_data_dir(app)
        .map_err(|_| "meeting_storage_unavailable")?
        .join("meetings");
    let store = MeetingStore::new(&root).map_err(str::to_owned)?;
    let recording_app = app.clone();
    let processing_app = app.clone();
    let event_app = app.clone();
    let vad_model = app
        .path()
        .resolve(
            "resources/models/silero_vad_v4.onnx",
            tauri::path::BaseDirectory::Resource,
        )
        .map_err(|_| "meeting_vad_init")?;
    let services = RuntimeServices {
        devices: Arc::new(super::capture::list_devices),
        capture: Arc::new(NativeCaptureFactory),
        acquire_recording: Arc::new(move || {
            let coordinator = recording_app
                .try_state::<crate::TranscriptionCoordinator>()
                .ok_or("meeting_not_ready")?;
            let permit = coordinator.try_claim_meeting_capture()?;
            let microphone = recording_app
                .try_state::<Arc<crate::managers::audio::AudioRecordingManager>>()
                .ok_or("meeting_not_ready")?;
            if microphone.is_recording() {
                return Err("meeting_dictation_busy".into());
            }
            Ok(Box::new(permit) as Box<dyn Send>)
        }),
        process: Arc::new(move |store, id, cancelled, progress| {
            #[cfg(all(target_os = "windows", target_arch = "x86_64", target_env = "msvc"))]
            {
                let manager = processing_app
                    .try_state::<Arc<crate::managers::transcription::TranscriptionManager>>()
                    .ok_or("meeting_model_missing")?;
                let _model_use = MeetingModelUse(Arc::clone(&manager));
                super::processing::process_meeting(
                    store,
                    id,
                    &vad_model,
                    cancelled,
                    |audio| {
                        manager
                            .transcribe_meeting(audio)
                            .map_err(transcription_error)
                    },
                    progress,
                )
            }
            #[cfg(not(all(target_os = "windows", target_arch = "x86_64", target_env = "msvc")))]
            {
                let _ = (store, id, cancelled, progress, &processing_app, &vad_model);
                Err("meeting_unsupported".into())
            }
        }),
        emit: Arc::new(move |summary| {
            let _ = event_app.emit("meeting-updated", summary);
        }),
    };
    app.manage(Arc::new(MeetingRuntime::new(store, root, services)?));
    Ok(())
}

fn runtime(app: &AppHandle) -> Result<Arc<MeetingRuntime>, String> {
    if !supported() {
        return Err("meeting_unsupported".into());
    }
    app.try_state::<Arc<MeetingRuntime>>()
        .map(|runtime| Arc::clone(&runtime))
        .ok_or_else(|| "meeting_storage_unavailable".into())
}
async fn worker<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|_| "meeting_worker_failed".to_string())?
}

#[tauri::command]
#[specta::specta]
pub async fn meeting_devices(app: AppHandle) -> Result<Vec<DeviceInfo>, String> {
    let _ = runtime(&app)?;
    worker(super::capture::list_devices).await
}
#[tauri::command]
#[specta::specta]
pub async fn meeting_list(app: AppHandle) -> Result<Vec<MeetingSummary>, String> {
    let runtime = runtime(&app)?;
    worker(move || runtime.list()).await
}
#[tauri::command]
#[specta::specta]
pub async fn meeting_current(app: AppHandle) -> Result<Option<MeetingSummary>, String> {
    runtime(&app)?.current()
}
#[tauri::command]
#[specta::specta]
pub async fn meeting_start(
    app: AppHandle,
    title: String,
    microphone_id: String,
    render_id: String,
) -> Result<MeetingSummary, String> {
    let runtime = runtime(&app)?;
    worker(move || runtime.start(&title, &microphone_id, &render_id)).await
}
#[tauri::command]
#[specta::specta]
pub async fn meeting_stop(app: AppHandle, id: String) -> Result<MeetingSummary, String> {
    let runtime = runtime(&app)?;
    worker(move || runtime.stop(&id)).await
}
#[tauri::command]
#[specta::specta]
pub async fn meeting_transcribe(app: AppHandle, id: String) -> Result<MeetingSummary, String> {
    let runtime = runtime(&app)?;
    worker(move || runtime.transcribe(&id)).await
}
#[tauri::command]
#[specta::specta]
pub async fn meeting_cancel_transcription(
    app: AppHandle,
    id: String,
) -> Result<MeetingSummary, String> {
    runtime(&app)?.cancel_transcription(&id)
}
#[tauri::command]
#[specta::specta]
pub async fn meeting_transcript(
    app: AppHandle,
    id: String,
) -> Result<Option<MeetingTranscript>, String> {
    let runtime = runtime(&app)?;
    worker(move || runtime.transcript(&id)).await
}
#[tauri::command]
#[specta::specta]
pub async fn meeting_delete(app: AppHandle, id: String) -> Result<(), String> {
    let runtime = runtime(&app)?;
    worker(move || runtime.delete(&id)).await
}
#[tauri::command]
#[specta::specta]
pub async fn meeting_export(
    app: AppHandle,
    id: String,
    destination: String,
    format: String,
    language: String,
) -> Result<(), String> {
    let runtime = runtime(&app)?;
    worker(move || runtime.export(&id, &PathBuf::from(destination), &format, &language)).await
}

pub fn shutdown(app: &AppHandle) {
    if let Some(runtime) = app.try_state::<Arc<MeetingRuntime>>() {
        let _ = runtime.shutdown();
    }
}

/// The existing scheduled maintenance also expires meeting audio and transcripts.
pub fn prune_data_root(root: &Path) -> Result<(), String> {
    let meetings = root.join("meetings");
    if !meetings.exists() {
        return Ok(());
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "meeting_clock")?
        .as_secs();
    MeetingStore::new(meetings)
        .and_then(|store| store.prune(now))
        .map(|_| ())
        .map_err(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::transcription_error;

    #[test]
    fn model_guidance_only_exposes_the_known_static_error() {
        assert_eq!(
            transcription_error(
                anyhow::anyhow!("meeting_model_unsupported").context("local processing")
            ),
            "meeting_model_unsupported"
        );
        assert_eq!(
            transcription_error(
                anyhow::anyhow!("private-model-path-and-native-error")
                    .context("meeting_model_unsupported")
            ),
            "meeting_transcription_failed"
        );
    }
}
