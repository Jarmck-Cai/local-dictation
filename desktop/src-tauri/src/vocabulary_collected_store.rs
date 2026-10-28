//! Independent local observations and durable human labels. No correction rules.
use rusqlite::{params, Connection, OptionalExtension};

pub const SCHEMA: &str = r#"
CREATE TABLE collected_terms (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    normalized TEXT NOT NULL UNIQUE,
    term TEXT NOT NULL,
    liked BOOLEAN NOT NULL DEFAULT 0 CHECK(liked IN (0,1)),
    deleted BOOLEAN NOT NULL DEFAULT 0 CHECK(deleted IN (0,1))
);
CREATE TABLE collected_term_observations (
    history_id INTEGER NOT NULL,
    term_id INTEGER NOT NULL,
    timestamp INTEGER NOT NULL,
    PRIMARY KEY(history_id, term_id)
);
CREATE INDEX collected_observations_term ON collected_term_observations(term_id, timestamp);
CREATE TABLE collected_history_state (history_id INTEGER PRIMARY KEY);
CREATE TABLE collected_revision (id INTEGER PRIMARY KEY CHECK(id=1), revision INTEGER NOT NULL);
INSERT INTO collected_revision VALUES(1,0);
CREATE TRIGGER collected_history_deleted AFTER DELETE ON transcription_history BEGIN
    DELETE FROM collected_term_observations WHERE history_id=OLD.id;
    DELETE FROM collected_history_state WHERE history_id=OLD.id;
    DELETE FROM collected_terms WHERE liked=0 AND deleted=0
        AND NOT EXISTS(SELECT 1 FROM collected_term_observations o WHERE o.term_id=collected_terms.id);
    UPDATE collected_revision SET revision=revision+1 WHERE id=1;
END;
CREATE TRIGGER collected_history_replaced AFTER UPDATE OF transcription_text, timestamp ON transcription_history
WHEN OLD.transcription_text IS NOT NEW.transcription_text OR OLD.timestamp IS NOT NEW.timestamp BEGIN
    DELETE FROM collected_term_observations WHERE history_id=OLD.id;
    DELETE FROM collected_history_state WHERE history_id=OLD.id;
    DELETE FROM collected_terms WHERE liked=0 AND deleted=0
        AND NOT EXISTS(SELECT 1 FROM collected_term_observations o WHERE o.term_id=collected_terms.id);
    UPDATE collected_revision SET revision=revision+1 WHERE id=1;
END;
"#;

pub type Result<T> = std::result::Result<T, &'static str>;
fn storage(_: rusqlite::Error) -> &'static str {
    "collected_terms_storage_unavailable"
}

#[derive(Debug)]
pub struct Term {
    pub id: i64,
    pub term: String,
    pub record_count: u32,
    pub last_seen: Option<i64>,
    pub liked: bool,
    pub deleted: bool,
}
#[derive(Debug)]
pub struct Page {
    pub entries: Vec<Term>,
    pub total: u32,
    pub offset: u32,
    pub limit: u32,
}

/// Literal, Unicode-bounded search, including % and _ as ordinary characters.
pub fn page(
    db: &Connection,
    query: Option<&str>,
    filter: Option<&str>,
    offset: Option<u32>,
    limit: Option<u32>,
) -> Result<Page> {
    let query = query.unwrap_or("").trim();
    if query.chars().count() > 256 {
        return Err("collected_terms_query_invalid");
    }
    let filter = filter.unwrap_or("all");
    if !matches!(filter, "all" | "liked" | "deleted") {
        return Err("collected_terms_filter_invalid");
    }
    let offset = offset.unwrap_or(0);
    let limit = limit.unwrap_or(30).clamp(1, 100);
    let tx = db.unchecked_transaction().map_err(storage)?;
    let cte = "WITH counts AS (SELECT term_id,COUNT(*) AS record_count,MAX(timestamp) AS last_seen FROM collected_term_observations GROUP BY term_id), visible AS (SELECT t.*,COALESCE(c.record_count,0) AS record_count,c.last_seen FROM collected_terms t LEFT JOIN counts c ON c.term_id=t.id WHERE instr(lower(t.term),lower(?1))>0 AND ((?2='deleted' AND t.deleted=1) OR (t.deleted=0 AND ((?2='all' AND (COALESCE(c.record_count,0)>=2 OR t.liked=1)) OR (?2='liked' AND t.liked=1)))))";
    let total: u32 = tx
        .query_row(
            &format!("{cte} SELECT COUNT(*) FROM visible"),
            params![query, filter],
            |r| r.get::<_, i64>(0),
        )
        .map_err(storage)?
        .clamp(0, i64::from(u32::MAX)) as u32;
    let entries = {
        let mut statement = tx.prepare(&format!("{cte} SELECT id,term,record_count,last_seen,liked,deleted FROM visible ORDER BY record_count DESC,last_seen DESC,id DESC LIMIT ?3 OFFSET ?4")).map_err(storage)?;
        let rows = statement
            .query_map(params![query, filter, limit, offset], |r| {
                Ok(Term {
                    id: r.get(0)?,
                    term: r.get(1)?,
                    record_count: r.get::<_, i64>(2)?.clamp(0, i64::from(u32::MAX)) as u32,
                    last_seen: r.get(3)?,
                    liked: r.get(4)?,
                    deleted: r.get(5)?,
                })
            })
            .map_err(storage)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(storage)?
    };
    tx.commit().map_err(storage)?;
    Ok(Page {
        entries,
        total,
        offset,
        limit,
    })
}

/// The queued source is checked in the SAME transaction as its replacement.
/// Deleted/replaced history discards the old task; no source body/hash is stored.
pub fn observe_snapshot(db: &mut Connection, id: i64, source: &str) -> Result<bool> {
    let terms = super::collected_core::extract(source);
    let tx = db.transaction().map_err(storage)?;
    let current: Option<(String, i64)> = tx
        .query_row(
            "SELECT transcription_text,timestamp FROM transcription_history WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(storage)?;
    let Some((text, timestamp)) = current else {
        return Ok(false);
    };
    if text != source {
        return Ok(false);
    }
    let observed: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM collected_history_state WHERE history_id=?1)",
            [id],
            |r| r.get(0),
        )
        .map_err(storage)?;
    if observed {
        return Ok(false);
    }
    tx.execute(
        "DELETE FROM collected_term_observations WHERE history_id=?1",
        [id],
    )
    .map_err(storage)?;
    for term in terms {
        let normalized = term.to_lowercase();
        tx.execute("INSERT INTO collected_terms(normalized,term) VALUES(?1,?2) ON CONFLICT(normalized) DO NOTHING",params![normalized,term]).map_err(storage)?;
        tx.execute("INSERT OR IGNORE INTO collected_term_observations(history_id,term_id,timestamp) SELECT ?1,id,?3 FROM collected_terms WHERE normalized=?2",params![id,normalized,timestamp]).map_err(storage)?;
    }
    tx.execute(
        "INSERT INTO collected_history_state(history_id) VALUES(?1)",
        [id],
    )
    .map_err(storage)?;
    tx.execute(
        "UPDATE collected_revision SET revision=revision+1 WHERE id=1",
        [],
    )
    .map_err(storage)?;
    tx.commit().map_err(storage)?;
    Ok(true)
}

pub fn observe_history(db: &mut Connection, id: i64) -> Result<bool> {
    let text: Option<String> = db
        .query_row(
            "SELECT transcription_text FROM transcription_history WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()
        .map_err(storage)?;
    match text {
        Some(text) => observe_snapshot(db, id, &text),
        None => Ok(false),
    }
}

/// One bounded batch; caller yields between batches and serializes background jobs.
pub fn backfill_batch(db: &mut Connection, limit: u32) -> Result<usize> {
    let rows = {
        let mut statement = db.prepare("SELECT id,transcription_text FROM transcription_history h WHERE NOT EXISTS(SELECT 1 FROM collected_history_state s WHERE s.history_id=h.id) ORDER BY id LIMIT ?1").map_err(storage)?;
        let rows = statement
            .query_map([limit.clamp(1, 32)], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(storage)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(storage)?
    };
    for (id, text) in &rows {
        observe_snapshot(db, *id, text)?;
    }
    Ok(rows.len())
}

pub fn set_liked(db: &mut Connection, id: i64, liked: bool) -> Result<()> {
    let tx = db.transaction().map_err(storage)?;
    if tx
        .execute(
            "UPDATE collected_terms SET liked=?2 WHERE id=?1",
            params![id, liked],
        )
        .map_err(storage)?
        == 0
    {
        return Err("collected_term_missing");
    }
    tx.execute("DELETE FROM collected_terms WHERE id=?1 AND liked=0 AND deleted=0 AND NOT EXISTS(SELECT 1 FROM collected_term_observations WHERE term_id=?1)",[id]).map_err(storage)?;
    tx.execute(
        "UPDATE collected_revision SET revision=revision+1 WHERE id=1",
        [],
    )
    .map_err(storage)?;
    tx.commit().map_err(storage)
}
pub fn set_deleted(db: &mut Connection, id: i64, deleted: bool) -> Result<()> {
    let tx = db.transaction().map_err(storage)?;
    if tx
        .execute(
            "UPDATE collected_terms SET deleted=?2 WHERE id=?1",
            params![id, deleted],
        )
        .map_err(storage)?
        == 0
    {
        return Err("collected_term_missing");
    }
    tx.execute(
        "UPDATE collected_revision SET revision=revision+1 WHERE id=1",
        [],
    )
    .map_err(storage)?;
    tx.commit().map_err(storage)
}

pub fn revision(db: &Connection) -> Result<i64> {
    db.query_row(
        "SELECT revision FROM collected_revision WHERE id=1",
        [],
        |r| r.get(0),
    )
    .map_err(storage)
}
