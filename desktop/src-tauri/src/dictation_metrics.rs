//! Software-stage timing only: never transcripts, credentials, or provider payloads.

#[path = "dictation_metrics_policy.rs"]
mod policy;

#[path = "dictation_resources.rs"]
mod resources;

pub(crate) use policy::{claim_recording_cancel, stopped_without_audio_outcome};

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};

static IO: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));
static SEQUENCE: AtomicU64 = AtomicU64::new(0);
const FILE_NAME: &str = "dictation-timings.json";
const MAX_BYTES: u64 = 512 * 1024;

#[derive(Clone, Serialize, Deserialize, specta::Type)]
pub struct DictationResourceSnapshot {
    pub stage: String,
    pub working_set_bytes: Option<u64>,
    pub process_cpu_ms: Option<u64>,
}

#[derive(Clone, Serialize, Deserialize, specta::Type)]
pub struct DictationTiming {
    pub id: String,
    pub history_id: Option<i64>,
    pub recorded_at_ms: u64,
    pub asr_ms: u64,
    pub cleanup_ms: u64,
    pub paste_ms: Option<u64>,
    pub total_ms: u64,
    pub cleanup_budget_ms: u64,
    pub mode: String,
    pub outcome: String,
    #[serde(default)]
    pub cleanup_reason: Option<String>,
    #[serde(default)]
    pub resource_snapshots: Option<Vec<DictationResourceSnapshot>>,
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}

pub fn elapsed_ms(start: Instant) -> u64 {
    start.elapsed().as_millis().min(u64::MAX as u128) as u64
}

impl DictationTiming {
    pub fn new(mode: &str) -> Self {
        let timestamp = now_ms();
        Self {
            id: format!("{timestamp}-{}", SEQUENCE.fetch_add(1, Ordering::Relaxed)),
            history_id: None,
            recorded_at_ms: timestamp,
            asr_ms: 0,
            cleanup_ms: 0,
            paste_ms: None,
            total_ms: 0,
            cleanup_budget_ms: 0,
            mode: mode.into(),
            outcome: "pending".into(),
            cleanup_reason: None,
            resource_snapshots: None,
        }
    }

    /// Called once when recording is released. Recording-only cancellations do
    /// not have a processing interval and therefore carry no resource samples.
    pub fn begin_resources(&mut self) {
        if let Some(reading) = resources::read_current_process() {
            self.resource_snapshots = Some(vec![DictationResourceSnapshot {
                stage: "started".into(),
                working_set_bytes: reading.working_set_bytes,
                process_cpu_ms: reading.process_cpu_ms,
            }]);
        }
    }

    pub fn snapshot_resources(&mut self, stage: &str) {
        let Some(snapshots) = self.resource_snapshots.as_mut() else {
            return;
        };
        // Bound metadata even if a future caller accidentally repeats a stage.
        if snapshots.len() >= 4 || snapshots.iter().any(|snapshot| snapshot.stage == stage) {
            return;
        }
        if let Some(reading) = resources::read_current_process() {
            snapshots.push(DictationResourceSnapshot {
                stage: stage.into(),
                working_set_bytes: reading.working_set_bytes,
                process_cpu_ms: reading.process_cpu_ms,
            });
        }
    }
}

fn read_store(root: &Path, current_ms: u64) -> Result<Vec<DictationTiming>, String> {
    let path = root.join(FILE_NAME);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err("timings_unavailable".into()),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_BYTES {
        return Err("invalid_timings".into());
    }
    let rows: Vec<DictationTiming> =
        serde_json::from_slice(&fs::read(path).map_err(|_| "timings_unavailable")?)
            .map_err(|_| "invalid_timings")?;
    let timestamps: Vec<u64> = rows.iter().map(|row| row.recorded_at_ms).collect();
    Ok(policy::retained_indices(&timestamps, current_ms)
        .into_iter()
        .map(|index| rows[index].clone())
        .collect())
}

fn prune_store(root: &Path, current_ms: u64) -> Result<Vec<DictationTiming>, String> {
    let rows = read_store(root, current_ms)?;
    if root.join(FILE_NAME).exists() {
        write_store(root, &rows)?;
    }
    Ok(rows)
}

fn write_store(root: &Path, rows: &[DictationTiming]) -> Result<(), String> {
    let temp = root.join("dictation-timings.tmp");
    let bytes = serde_json::to_vec(rows).map_err(|_| "timings_unavailable")?;
    fs::write(&temp, bytes).map_err(|_| "timings_unavailable")?;
    fs::rename(&temp, root.join(FILE_NAME)).map_err(|_| "timings_unavailable")?;
    Ok(())
}

/// Normal application startup/daily maintenance only; audio-only scheduled
/// maintenance deliberately never reads or modifies timing/text metadata.
pub fn prune(app: &AppHandle) {
    let attempt = || -> Result<(), String> {
        let _guard = IO.lock().map_err(|_| "timings_unavailable")?;
        let root = crate::portable::app_data_dir(app).map_err(|_| "timings_unavailable")?;
        prune_store(&root, now_ms())?;
        Ok(())
    };
    if attempt().is_err() {
        log::warn!("Could not prune dictation timing metadata");
    }
}

#[tauri::command]
#[specta::specta]
pub fn get_dictation_metrics(app: AppHandle) -> Result<Vec<DictationTiming>, String> {
    let _guard = IO.lock().map_err(|_| "timings_unavailable")?;
    let root = crate::portable::app_data_dir(&app).map_err(|_| "timings_unavailable")?;
    prune_store(&root, now_ms())
}

/// Persistence and event delivery happen off the text injection path. Durations
/// are captured before enqueueing, so disk/UI scheduling does not inflate them.
pub fn record(app: &AppHandle, mut timing: DictationTiming) {
    timing.snapshot_resources("finished");
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let persist = || -> Result<(), String> {
            let _guard = IO.lock().map_err(|_| "timings_unavailable")?;
            let root = crate::portable::app_data_dir(&app).map_err(|_| "timings_unavailable")?;
            let mut rows = read_store(&root, now_ms()).unwrap_or_default();
            rows.retain(|old| old.id != timing.id);
            rows.push(timing.clone());
            let timestamps: Vec<u64> = rows.iter().map(|row| row.recorded_at_ms).collect();
            let retained: Vec<DictationTiming> = policy::retained_indices(&timestamps, now_ms())
                .into_iter()
                .map(|index| rows[index].clone())
                .collect();
            write_store(&root, &retained)?;
            Ok(())
        };
        if persist().is_err() {
            log::warn!("Could not persist dictation timing metadata");
        }
        let _ = app.emit("dictation-timing", &timing);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pruning_physically_removes_expired_rows_without_another_recording() {
        let root = std::env::temp_dir().join(format!(
            "local-dictation-timings-{}-{}-{}",
            std::process::id(),
            now_ms(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let current = policy::RETENTION_MS + 10;
        let mut expired = DictationTiming::new("claude");
        expired.recorded_at_ms = 9;
        let mut recent = DictationTiming::new("off");
        recent.recorded_at_ms = current;
        fs::write(
            root.join(FILE_NAME),
            serde_json::to_vec(&vec![expired, recent]).unwrap(),
        )
        .unwrap();
        let visible = prune_store(&root, current).unwrap();
        let disk: Vec<DictationTiming> =
            serde_json::from_slice(&fs::read(root.join(FILE_NAME)).unwrap()).unwrap();
        fs::remove_file(root.join(FILE_NAME)).unwrap();
        fs::remove_dir(root).unwrap();
        assert_eq!(visible.len(), 1);
        assert_eq!(
            disk.len(),
            1,
            "expiry must change storage, not only the response"
        );
        assert_eq!(disk[0].recorded_at_ms, current);
    }

    #[test]
    fn paste_failure_preserves_the_independent_cleanup_reason() {
        let mut json = serde_json::to_value(DictationTiming::new("claude")).unwrap();
        json["cleanup_reason"] = serde_json::json!("cleanup_timeout");
        let mut timing: DictationTiming = serde_json::from_value(json).unwrap();
        timing.outcome = "paste_failed".into();
        let stored = serde_json::to_value(timing).unwrap();
        assert_eq!(stored["outcome"], "paste_failed");
        assert_eq!(stored["cleanup_reason"], "cleanup_timeout");
    }

    #[test]
    fn old_metadata_without_resource_fields_remains_readable() {
        let mut json = serde_json::to_value(DictationTiming::new("off")).unwrap();
        json.as_object_mut().unwrap().remove("resource_snapshots");
        let timing: DictationTiming = serde_json::from_value(json).unwrap();
        assert!(timing.resource_snapshots.is_none());
        assert_eq!(timing.mode, "off");
    }
}
