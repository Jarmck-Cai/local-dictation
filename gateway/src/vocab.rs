//! SQLite-backed cross-device vocabulary synchronization store.
//!
//! Only vocabulary rules, technical terms, counts, and timestamps are stored here.
//! Audio bytes and full dictation transcripts are NEVER stored on the gateway.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_SYNC_BATCH: usize = 2_000;
const MAX_TOTAL_ROWS: i64 = 5_000;
const MAX_ALIASES: usize = 20;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS gateway_meta (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    revision INTEGER NOT NULL DEFAULT 0
);
INSERT OR IGNORE INTO gateway_meta (id, revision) VALUES (1, 0);

CREATE TABLE IF NOT EXISTS gateway_vocabulary (
    normalized TEXT PRIMARY KEY,
    term TEXT NOT NULL,
    kind TEXT NOT NULL,
    aliases_json TEXT NOT NULL DEFAULT '[]',
    context TEXT,
    count INTEGER NOT NULL DEFAULT 1,
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    updated_at_ms INTEGER NOT NULL,
    revision INTEGER NOT NULL,
    updated_by TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_gateway_vocab_revision ON gateway_vocabulary(revision);
CREATE INDEX IF NOT EXISTS idx_gateway_vocab_active ON gateway_vocabulary(enabled, kind, count DESC);
"#;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VocabularyKind {
    Confirmed,
    Custom,
    Liked,
    Frequent,
}

impl VocabularyKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Confirmed => "confirmed",
            Self::Custom => "custom",
            Self::Liked => "liked",
            Self::Frequent => "frequent",
        }
    }

    pub fn parse_kind(value: &str) -> Self {
        match value {
            "confirmed" => Self::Confirmed,
            "custom" => Self::Custom,
            "liked" => Self::Liked,
            _ => Self::Frequent,
        }
    }

    fn priority(self) -> u8 {
        match self {
            Self::Confirmed => 4,
            Self::Custom => 3,
            Self::Liked => 2,
            Self::Frequent => 1,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VocabularySyncEntry {
    pub term: String,
    #[serde(default = "default_kind")]
    pub kind: VocabularyKind,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub context: Option<String>,
    #[serde(default = "default_count")]
    pub count: u32,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub updated_at_ms: i64,
    #[serde(default)]
    pub revision: i64,
}

fn default_kind() -> VocabularyKind {
    VocabularyKind::Frequent
}

fn default_count() -> u32 {
    1
}

fn default_enabled() -> bool {
    true
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct VocabularySyncRequest {
    #[serde(default)]
    pub since_revision: i64,
    #[serde(default)]
    pub entries: Vec<VocabularySyncEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VocabularySyncResponse {
    pub revision: i64,
    pub total_active: u32,
    pub upserted: u32,
    pub entries: Vec<VocabularySyncEntry>,
}

pub struct VocabularyStore {
    conn: Mutex<Connection>,
}

impl VocabularyStore {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("failed to create gateway db directory: {e}"))?;
            }
        }
        let conn =
            Connection::open(path).map_err(|e| format!("failed to open gateway sqlite db: {e}"))?;
        Self::init_connection(conn)
    }

    pub fn open_in_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory()
            .map_err(|e| format!("failed to open in-memory db: {e}"))?;
        Self::init_connection(conn)
    }

    fn init_connection(conn: Connection) -> Result<Self, String> {
        conn.busy_timeout(std::time::Duration::from_millis(1_000))
            .map_err(|e| format!("failed to set busy_timeout: {e}"))?;
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")
            .map_err(|e| format!("failed to set sqlite pragmas: {e}"))?;
        conn.execute_batch(SCHEMA)
            .map_err(|e| format!("failed to initialize vocabulary schema: {e}"))?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn summary(&self) -> Result<(i64, u32), &'static str> {
        let conn = self.conn.lock().map_err(|_| "vocab_db_locked")?;
        let revision: i64 = conn
            .query_row("SELECT revision FROM gateway_meta WHERE id = 1", [], |r| {
                r.get(0)
            })
            .map_err(|_| "vocab_db_read_failed")?;
        let active: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM gateway_vocabulary WHERE enabled = 1",
                [],
                |r| r.get(0),
            )
            .map_err(|_| "vocab_db_read_failed")?;
        Ok((revision, active.clamp(0, i64::from(u32::MAX)) as u32))
    }

    pub fn list_since(&self, since_revision: i64) -> Result<VocabularySyncResponse, &'static str> {
        let conn = self.conn.lock().map_err(|_| "vocab_db_locked")?;
        Self::read_delta(&conn, since_revision.max(0), 0)
    }

    pub fn sync(
        &self,
        device_id: &str,
        req: VocabularySyncRequest,
    ) -> Result<VocabularySyncResponse, &'static str> {
        if req.entries.len() > MAX_SYNC_BATCH {
            return Err("vocabulary_sync_batch_too_large");
        }
        let now = now_ms();
        let mut conn = self.conn.lock().map_err(|_| "vocab_db_locked")?;
        let tx = conn.transaction().map_err(|_| "vocab_db_tx_failed")?;

        let mut current_rev: i64 = tx
            .query_row("SELECT revision FROM gateway_meta WHERE id = 1", [], |r| {
                r.get(0)
            })
            .map_err(|_| "vocab_db_read_failed")?;
        let mut total_rows: i64 = tx
            .query_row("SELECT COUNT(*) FROM gateway_vocabulary", [], |r| r.get(0))
            .map_err(|_| "vocab_db_read_failed")?;

        let mut upserted: u32 = 0;

        for incoming in req.entries {
            let Some(cleaned) = sanitize_entry(incoming, now) else {
                continue;
            };
            let normalized = cleaned.term.to_lowercase();

            struct ExistingRow {
                term: String,
                kind_str: String,
                aliases_json: String,
                context: Option<String>,
                count: u32,
                enabled: bool,
                updated_at_ms: i64,
            }

            let existing: Option<ExistingRow> = tx
                .query_row(
                    "SELECT term, kind, aliases_json, context, count, enabled, updated_at_ms
                     FROM gateway_vocabulary WHERE normalized = ?1",
                    [&normalized],
                    |r| {
                        Ok(ExistingRow {
                            term: r.get(0)?,
                            kind_str: r.get(1)?,
                            aliases_json: r.get(2)?,
                            context: r.get(3)?,
                            count: r.get::<_, i64>(4)?.clamp(1, i64::from(u32::MAX)) as u32,
                            enabled: r.get::<_, i64>(5)? != 0,
                            updated_at_ms: r.get(6)?,
                        })
                    },
                )
                .optional()
                .map_err(|_| "vocab_db_read_failed")?;

            match existing {
                None => {
                    if total_rows >= MAX_TOTAL_ROWS {
                        return Err("vocabulary_capacity_exceeded");
                    }
                    current_rev = current_rev.saturating_add(1);
                    let aliases_json =
                        serde_json::to_string(&cleaned.aliases).unwrap_or_else(|_| "[]".into());
                    tx.execute(
                        "INSERT INTO gateway_vocabulary
                         (normalized, term, kind, aliases_json, context, count, enabled, updated_at_ms, revision, updated_by)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                        params![
                            normalized,
                            cleaned.term,
                            cleaned.kind.as_str(),
                            aliases_json,
                            cleaned.context,
                            cleaned.count,
                            i64::from(cleaned.enabled),
                            cleaned.updated_at_ms,
                            current_rev,
                            device_id,
                        ],
                    )
                    .map_err(|_| "vocab_db_write_failed")?;
                    total_rows += 1;
                    upserted += 1;
                }
                Some(ex) => {
                    let ex_kind = VocabularyKind::parse_kind(&ex.kind_str);
                    let ex_aliases: Vec<String> =
                        serde_json::from_str(&ex.aliases_json).unwrap_or_default();

                    let incoming_newer = cleaned.updated_at_ms >= ex.updated_at_ms;
                    let incoming_higher_kind =
                        cleaned.enabled && cleaned.kind.priority() > ex_kind.priority();

                    let merged_kind = if incoming_higher_kind
                        || (incoming_newer && !cleaned.enabled)
                        || ex_kind.priority() < cleaned.kind.priority()
                    {
                        cleaned.kind
                    } else {
                        ex_kind
                    };

                    let merged_term = if incoming_higher_kind
                        || (incoming_newer && cleaned.kind.priority() >= ex_kind.priority())
                    {
                        cleaned.term
                    } else {
                        ex.term.clone()
                    };

                    let merged_enabled = if incoming_newer {
                        cleaned.enabled
                    } else if incoming_higher_kind {
                        true
                    } else {
                        ex.enabled
                    };

                    let merged_context = if incoming_newer && cleaned.context.is_some() {
                        cleaned.context
                    } else {
                        ex.context.clone().or(cleaned.context)
                    };

                    let merged_aliases = merge_aliases(&ex_aliases, &cleaned.aliases);
                    let merged_count = ex.count.max(cleaned.count);
                    let merged_updated_at = ex.updated_at_ms.max(cleaned.updated_at_ms);

                    let unchanged = merged_term == ex.term
                        && merged_kind == ex_kind
                        && merged_aliases == ex_aliases
                        && merged_context == ex.context
                        && merged_count == ex.count
                        && merged_enabled == ex.enabled;

                    if unchanged {
                        continue;
                    }

                    current_rev = current_rev.saturating_add(1);
                    let aliases_json =
                        serde_json::to_string(&merged_aliases).unwrap_or_else(|_| "[]".into());
                    tx.execute(
                        "UPDATE gateway_vocabulary
                         SET term = ?2, kind = ?3, aliases_json = ?4, context = ?5,
                             count = ?6, enabled = ?7, updated_at_ms = ?8, revision = ?9, updated_by = ?10
                         WHERE normalized = ?1",
                        params![
                            normalized,
                            merged_term,
                            merged_kind.as_str(),
                            aliases_json,
                            merged_context,
                            merged_count,
                            i64::from(merged_enabled),
                            merged_updated_at,
                            current_rev,
                            device_id,
                        ],
                    )
                    .map_err(|_| "vocab_db_write_failed")?;
                    upserted += 1;
                }
            }
        }

        if upserted > 0 {
            tx.execute(
                "UPDATE gateway_meta SET revision = ?1 WHERE id = 1",
                [current_rev],
            )
            .map_err(|_| "vocab_db_write_failed")?;
        }

        let response = Self::read_delta(&tx, req.since_revision.max(0), upserted)?;
        tx.commit().map_err(|_| "vocab_db_commit_failed")?;
        Ok(response)
    }

    /// Select active, prompt-safe vocabulary terms ordered by importance (`confirmed` >
    /// `custom` > `liked` > `frequent`, then `count DESC`, `updated_at_ms DESC`),
    /// merged with optional per-request `hint_terms`.
    pub fn select_prompt_terms(&self, hint_terms: &[String], max_terms: usize) -> Vec<String> {
        let mut result = Vec::with_capacity(max_terms);
        let mut seen_lower = std::collections::HashSet::with_capacity(max_terms);

        let mut push_term = |candidate: &str| -> bool {
            if result.len() >= max_terms {
                return true;
            }
            if let Some(clean) = sanitize_prompt_term(candidate) {
                let lower = clean.to_lowercase();
                if seen_lower.insert(lower) {
                    result.push(clean);
                }
            }
            result.len() >= max_terms
        };

        for hint in hint_terms {
            if push_term(hint) {
                return result;
            }
        }

        if let Ok(conn) = self.conn.lock() {
            if let Ok(mut stmt) = conn.prepare(
                "SELECT term FROM gateway_vocabulary
                 WHERE enabled = 1
                 ORDER BY CASE kind
                     WHEN 'confirmed' THEN 4
                     WHEN 'custom' THEN 3
                     WHEN 'liked' THEN 2
                     ELSE 1
                 END DESC, count DESC, updated_at_ms DESC
                 LIMIT ?1",
            ) {
                if let Ok(rows) = stmt.query_map([max_terms as i64 * 2], |r| r.get::<_, String>(0))
                {
                    for term in rows.flatten() {
                        if push_term(&term) {
                            break;
                        }
                    }
                }
            }
        }

        result
    }

    fn read_delta(
        conn: &Connection,
        since_revision: i64,
        upserted: u32,
    ) -> Result<VocabularySyncResponse, &'static str> {
        let revision: i64 = conn
            .query_row("SELECT revision FROM gateway_meta WHERE id = 1", [], |r| {
                r.get(0)
            })
            .map_err(|_| "vocab_db_read_failed")?;
        let total_active: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM gateway_vocabulary WHERE enabled = 1",
                [],
                |r| r.get(0),
            )
            .map_err(|_| "vocab_db_read_failed")?;

        let mut stmt = conn
            .prepare(
                "SELECT term, kind, aliases_json, context, count, enabled, updated_at_ms, revision
                 FROM gateway_vocabulary
                 WHERE revision > ?1
                 ORDER BY revision ASC",
            )
            .map_err(|_| "vocab_db_read_failed")?;

        let rows = stmt
            .query_map([since_revision], |r| {
                let aliases_json: String = r.get(2)?;
                let aliases: Vec<String> = serde_json::from_str(&aliases_json).unwrap_or_default();
                Ok(VocabularySyncEntry {
                    term: r.get(0)?,
                    kind: VocabularyKind::parse_kind(&r.get::<_, String>(1)?),
                    aliases,
                    context: r.get(3)?,
                    count: r.get::<_, i64>(4)?.clamp(1, i64::from(u32::MAX)) as u32,
                    enabled: r.get::<_, i64>(5)? != 0,
                    updated_at_ms: r.get(6)?,
                    revision: r.get(7)?,
                })
            })
            .map_err(|_| "vocab_db_read_failed")?;

        let entries = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "vocab_db_read_failed")?;

        Ok(VocabularySyncResponse {
            revision,
            total_active: total_active.clamp(0, i64::from(u32::MAX)) as u32,
            upserted,
            entries,
        })
    }
}

pub fn sanitize_prompt_term(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let count = trimmed.chars().count();
    if count == 0
        || count > 64
        || trimmed
            .chars()
            .any(|c| c.is_control() || matches!(c, '<' | '>' | '\r' | '\n' | '\0'))
    {
        return None;
    }
    Some(trimmed.to_string())
}

fn sanitize_entry(
    mut entry: VocabularySyncEntry,
    fallback_now_ms: i64,
) -> Option<VocabularySyncEntry> {
    entry.term = sanitize_prompt_term(&entry.term)?;
    entry.aliases = entry
        .aliases
        .into_iter()
        .filter_map(|alias| sanitize_prompt_term(&alias))
        .filter(|alias| !alias.eq_ignore_ascii_case(&entry.term))
        .fold(Vec::new(), |mut acc, alias| {
            if acc.len() < MAX_ALIASES
                && !acc
                    .iter()
                    .any(|existing| existing.eq_ignore_ascii_case(&alias))
            {
                acc.push(alias);
            }
            acc
        });
    entry.context = entry
        .context
        .and_then(|ctx| sanitize_prompt_term(&ctx))
        .filter(|ctx| (2..=24).contains(&ctx.chars().count()));
    entry.count = entry.count.max(1);
    if entry.updated_at_ms <= 0 {
        entry.updated_at_ms = fallback_now_ms;
    }
    Some(entry)
}

fn merge_aliases(existing: &[String], incoming: &[String]) -> Vec<String> {
    let mut merged = Vec::new();
    for alias in existing.iter().chain(incoming.iter()) {
        if merged.len() >= MAX_ALIASES {
            break;
        }
        if !merged
            .iter()
            .any(|item: &String| item.eq_ignore_ascii_case(alias))
        {
            merged.push(alias.clone());
        }
    }
    merged
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_deduplicates_merges_across_devices_and_supports_incremental_pull() {
        let store = VocabularyStore::open_in_memory().unwrap();

        // Desktop pushes frequent + confirmed technical terms
        let desktop_res = store
            .sync(
                "desktop",
                VocabularySyncRequest {
                    since_revision: 0,
                    entries: vec![
                        VocabularySyncEntry {
                            term: "claude code".into(),
                            kind: VocabularyKind::Frequent,
                            aliases: vec![],
                            context: None,
                            count: 5,
                            enabled: true,
                            updated_at_ms: 1_000,
                            revision: 0,
                        },
                        VocabularySyncEntry {
                            term: "SenseVoiceSmall".into(),
                            kind: VocabularyKind::Liked,
                            aliases: vec![],
                            context: None,
                            count: 3,
                            enabled: true,
                            updated_at_ms: 1_000,
                            revision: 0,
                        },
                    ],
                },
            )
            .unwrap();
        assert_eq!(desktop_res.revision, 2);
        assert_eq!(desktop_res.total_active, 2);

        // Laptop pulls from revision 0 -> receives Desktop's 2 terms
        let laptop_pull = store.list_since(0).unwrap();
        assert_eq!(laptop_pull.entries.len(), 2);

        // Laptop promotes "Claude Code" to Confirmed with canonical casing and alias
        let laptop_sync = store
            .sync(
                "laptop",
                VocabularySyncRequest {
                    since_revision: laptop_pull.revision,
                    entries: vec![VocabularySyncEntry {
                        term: "Claude Code".into(),
                        kind: VocabularyKind::Confirmed,
                        aliases: vec!["克劳德 Code".into()],
                        context: Some("终端".into()),
                        count: 2,
                        enabled: true,
                        updated_at_ms: 2_000,
                        revision: 0,
                    }],
                },
            )
            .unwrap();
        assert_eq!(laptop_sync.revision, 3);
        assert_eq!(laptop_sync.total_active, 2);
        assert_eq!(laptop_sync.entries.len(), 1);
        assert_eq!(laptop_sync.entries[0].term, "Claude Code");
        assert_eq!(laptop_sync.entries[0].kind, VocabularyKind::Confirmed);
        assert_eq!(laptop_sync.entries[0].count, 5); // preserved higher count!
        assert_eq!(laptop_sync.entries[0].aliases, vec!["克劳德 Code"]);

        // Desktop incremental pull since revision 2 receives ONLY the updated entry
        let desktop_delta = store.list_since(2).unwrap();
        assert_eq!(desktop_delta.entries.len(), 1);
        assert_eq!(desktop_delta.entries[0].term, "Claude Code");

        // Prompt selector puts Confirmed before Liked and strips unsafe chars
        let prompt_terms = store.select_prompt_terms(&["Tauri".into(), "bad<script>".into()], 16);
        assert_eq!(
            prompt_terms,
            vec!["Tauri", "Claude Code", "SenseVoiceSmall"]
        );
    }
}
