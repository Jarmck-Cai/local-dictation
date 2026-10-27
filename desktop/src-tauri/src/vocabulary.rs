//! Local-only, explicitly reviewed vocabulary learning. No audio or network access.
#[path = "vocabulary_core.rs"]
mod core;
#[path = "vocabulary_store.rs"]
mod store;

use crate::managers::history::HistoryEntry;
use rusqlite::Connection;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

pub(crate) const SCHEMA: &str = store::SCHEMA;

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
            store::view(connection)
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
    let proposals = core::extract_terms(&entry.transcription_text);
    with_connection(app, |connection| {
        store::observe(connection, entry.id, &proposals)
    })?;
    let _ = app.emit("vocabulary-updated", ());
    Ok(())
}

fn confirmed_rules(app: &AppHandle) -> Vec<core::Rule> {
    with_connection(app, |connection| {
        let transaction = connection
            .transaction()
            .map_err(|_| "vocabulary_storage_unavailable")?;
        let rules = store::terms(&transaction)?
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
    core::apply_rules(raw, &confirmed_rules(app))
}

/// Return only confirmed vocabulary matching this transcript or a contextual alias.
pub fn relevant_terms(app: &AppHandle, raw: &str) -> Vec<String> {
    core::relevant_terms(raw, &confirmed_rules(app))
}
