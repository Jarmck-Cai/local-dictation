//! Local-only, explicitly reviewed vocabulary learning. No audio or network access.
#[path = "vocabulary_collected_core.rs"]
mod collected_core;
#[path = "vocabulary_collected_runtime.rs"]
mod collected_runtime;
#[path = "vocabulary_collected_store.rs"]
mod collected_store;
#[cfg(test)]
#[path = "vocabulary_collected_tests.rs"]
mod collected_tests;
#[path = "vocabulary_core.rs"]
mod core;
#[path = "vocabulary_store.rs"]
mod store;

use crate::managers::history::HistoryEntry;
use rusqlite::Connection;
use serde::Serialize;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter};

pub(crate) const SCHEMA: &str = store::SCHEMA;
pub(crate) const MANAGEMENT_SCHEMA: &str = store::MANAGEMENT_SCHEMA;
pub(crate) const COLLECTED_SCHEMA: &str = collected_store::SCHEMA;
pub use collected_runtime::schedule_collected_terms_refresh;

// A fresh application process starts empty. No transcript body is retained.
static RECENT_CORRECTIONS: Mutex<core::RecentCorrections> = Mutex::new(core::RecentCorrections {
    hits: Vec::new(),
    recorded_at_ms: None,
});

#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct VocabularyCorrectionHit {
    pub term_id: i64,
    pub term: String,
    pub from: String,
    pub count: u32,
}

#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct VocabularyCorrectionSnapshot {
    pub recorded_at_ms: Option<u64>,
    pub hits: Vec<VocabularyCorrectionHit>,
}

fn correction_snapshot(recent: &core::RecentCorrections) -> VocabularyCorrectionSnapshot {
    VocabularyCorrectionSnapshot {
        recorded_at_ms: recent.recorded_at_ms,
        hits: recent
            .hits
            .iter()
            .map(|hit| VocabularyCorrectionHit {
                term_id: hit.term_id,
                term: hit.term.clone(),
                from: hit.from.clone(),
                count: hit.count,
            })
            .collect(),
    }
}

#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct VocabularyCandidate {
    pub id: i64,
    pub term: String,
    pub replacement_from: Option<String>,
    pub context: Option<String>,
    pub source: String,
    pub occurrences: u32,
}

#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct VocabularyTerm {
    pub id: i64,
    pub term: String,
    pub aliases: Vec<String>,
    pub context: Option<String>,
    pub enabled: bool,
}

#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct VocabularyView {
    pub candidates: Vec<VocabularyCandidate>,
    pub entries: Vec<VocabularyTerm>,
}

impl From<store::View> for VocabularyView {
    fn from(value: store::View) -> Self {
        Self {
            candidates: value
                .candidates
                .into_iter()
                .filter(|candidate| candidate.proposal.source != "transcript")
                .map(|candidate| VocabularyCandidate {
                    id: candidate.id,
                    term: candidate.proposal.term,
                    replacement_from: candidate.proposal.replacement_from,
                    context: candidate.proposal.context,
                    source: candidate.proposal.source,
                    occurrences: candidate.proposal.occurrences,
                })
                .collect(),
            entries: value
                .entries
                .into_iter()
                .map(|entry| VocabularyTerm {
                    id: entry.id,
                    term: entry.rule.term,
                    aliases: entry.rule.aliases,
                    context: entry.rule.context,
                    enabled: entry.enabled,
                })
                .collect(),
        }
    }
}

fn with_connection<T>(
    app: &AppHandle,
    operation: impl FnOnce(&mut Connection) -> store::Result<T>,
) -> Result<T, String> {
    let path = crate::portable::app_data_dir(app)
        .map_err(|_| "vocabulary_storage_unavailable".to_owned())?
        .join("history.db");
    let mut connection =
        Connection::open(path).map_err(|_| "vocabulary_storage_unavailable".to_owned())?;
    // A busy history writer must not introduce seconds of additional waiting.
    connection
        .busy_timeout(std::time::Duration::from_millis(250))
        .map_err(|_| "vocabulary_storage_unavailable".to_owned())?;
    operation(&mut connection).map_err(str::to_owned)
}

async fn command(
    app: AppHandle,
    operation: impl FnOnce(&mut Connection) -> store::Result<()> + Send + 'static,
    changed: bool,
) -> Result<VocabularyView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let view = with_connection(&app, |connection| {
            operation(connection)?;
            store::reviewable_view(connection)
        })?;
        if changed {
            let _ = app.emit("vocabulary-updated", ());
        }
        Ok(view.into())
    })
    .await
    .map_err(|_| "vocabulary_unavailable".to_owned())?
}

#[tauri::command]
#[specta::specta]
pub async fn get_vocabulary(app: AppHandle) -> Result<VocabularyView, String> {
    command(app, |_| Ok(()), false).await
}

#[tauri::command]
#[specta::specta]
pub async fn review_vocabulary_candidate(
    app: AppHandle,
    id: i64,
    accept: bool,
) -> Result<VocabularyView, String> {
    command(
        app,
        move |connection| store::review(connection, id, accept),
        true,
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn delete_learned_term(app: AppHandle, id: i64) -> Result<VocabularyView, String> {
    command(
        app,
        move |connection| store::delete_term(connection, id),
        true,
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn update_learned_term(
    app: AppHandle,
    id: i64,
    term: String,
    aliases: Vec<String>,
    context: Option<String>,
) -> Result<VocabularyView, String> {
    command(
        app,
        move |connection| store::update_term(connection, id, &term, &aliases, context.as_deref()),
        true,
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn set_learned_term_enabled(
    app: AppHandle,
    id: i64,
    enabled: bool,
) -> Result<VocabularyView, String> {
    command(
        app,
        move |connection| store::set_enabled(connection, id, enabled),
        true,
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub fn get_vocabulary_correction_snapshot() -> Result<VocabularyCorrectionSnapshot, String> {
    let recent = RECENT_CORRECTIONS
        .lock()
        .map_err(|_| "vocabulary_unavailable".to_owned())?;
    Ok(correction_snapshot(&recent))
}

/// Clear at the start of a dictation, including attempts that never reach ASR.
pub fn clear_recent_corrections(app: &AppHandle) {
    if let Ok(mut recent) = RECENT_CORRECTIONS.lock() {
        *recent = core::RecentCorrections::default();
        let snapshot = correction_snapshot(&recent);
        drop(recent);
        let _ = app.emit("vocabulary-correction-updated", snapshot);
    }
}

#[tauri::command]
#[specta::specta]
pub async fn submit_correction_sample(
    app: AppHandle,
    history_id: i64,
    corrected_text: String,
) -> Result<VocabularyView, String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs().min(i64::MAX as u64) as i64);
    command(
        app,
        move |connection| store::submit(connection, history_id, &corrected_text, now),
        true,
    )
    .await
}

/// Called after history save, off the input delivery path. Re-observation is idempotent.
pub fn observe_transcript(app: &AppHandle, entry: &HistoryEntry) -> Result<(), String> {
    let changed = with_connection(app, |connection| {
        collected_store::observe_snapshot(connection, entry.id, &entry.transcription_text)
    })?;
    if changed {
        let _ = app.emit("collected-terms-updated", ());
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct CollectedTerm {
    pub id: i64,
    pub term: String,
    pub record_count: u32,
    pub last_seen: Option<i64>,
    pub liked: bool,
    pub deleted: bool,
}

#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct CollectedTermsPage {
    pub entries: Vec<CollectedTerm>,
    pub total: u32,
    pub offset: u32,
    pub limit: u32,
}

#[tauri::command]
#[specta::specta]
pub async fn get_collected_terms(
    app: AppHandle,
    query: Option<String>,
    filter: Option<String>,
    offset: Option<u32>,
    limit: Option<u32>,
) -> Result<CollectedTermsPage, String> {
    // A read returns the current snapshot; reconciliation remains off the input path.
    schedule_collected_terms_refresh(&app);
    tauri::async_runtime::spawn_blocking(move || {
        let page = with_connection(&app, |db| {
            collected_store::page(db, query.as_deref(), filter.as_deref(), offset, limit)
        })?;
        Ok(CollectedTermsPage {
            entries: page
                .entries
                .into_iter()
                .map(|term| CollectedTerm {
                    id: term.id,
                    term: term.term,
                    record_count: term.record_count,
                    last_seen: term.last_seen,
                    liked: term.liked,
                    deleted: term.deleted,
                })
                .collect(),
            total: page.total,
            offset: page.offset,
            limit: page.limit,
        })
    })
    .await
    .map_err(|_| "collected_terms_unavailable".to_owned())?
}

async fn collected_mutation(
    app: AppHandle,
    operation: impl FnOnce(&mut Connection) -> store::Result<()> + Send + 'static,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        with_connection(&app, operation)?;
        let _ = app.emit("collected-terms-updated", ());
        Ok(())
    })
    .await
    .map_err(|_| "collected_terms_unavailable".to_owned())?
}

#[tauri::command]
#[specta::specta]
pub async fn set_collected_term_liked(app: AppHandle, id: i64, liked: bool) -> Result<(), String> {
    collected_mutation(app, move |db| collected_store::set_liked(db, id, liked)).await
}

#[tauri::command]
#[specta::specta]
pub async fn set_collected_term_deleted(
    app: AppHandle,
    id: i64,
    deleted: bool,
) -> Result<(), String> {
    collected_mutation(app, move |db| collected_store::set_deleted(db, id, deleted)).await
}

fn confirmed_rules(app: &AppHandle) -> Vec<core::Rule> {
    with_connection(app, |connection| {
        let transaction = connection
            .transaction()
            .map_err(|_| "vocabulary_storage_unavailable")?;
        let rules = store::active_terms(&transaction)?
            .into_iter()
            .map(|term| term.rule)
            .collect();
        transaction
            .commit()
            .map_err(|_| "vocabulary_storage_unavailable")?;
        Ok(rules)
    })
    .unwrap_or_default()
}

/// Apply only confirmed, nonambiguous aliases to the original text, once.
/// A storage failure preserves the complete local transcript.
pub fn apply_confirmed(app: &AppHandle, raw: &str) -> String {
    let result = core::apply_rules_with_hits(raw, &confirmed_rules(app));
    // Replacing the snapshot even for no hits prevents stale corrections from
    // being attributed to a later dictation. Storage failure follows this path too.
    if let Ok(mut recent) = RECENT_CORRECTIONS.lock() {
        recent.record(result.hits);
        let snapshot = correction_snapshot(&recent);
        drop(recent);
        let _ = app.emit("vocabulary-correction-updated", snapshot);
    }
    result.text
}

/// Return only confirmed vocabulary matching this transcript or a contextual alias.
pub fn relevant_terms(app: &AppHandle, raw: &str) -> Vec<String> {
    core::relevant_terms(raw, &confirmed_rules(app))
}
