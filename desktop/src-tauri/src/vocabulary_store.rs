//! SQLite persistence shared by the application and isolated in-memory checks.
use super::core::{Proposal, Rule};
use rusqlite::{params, Connection, OptionalExtension};

pub const SCHEMA: &str = r#"
CREATE TABLE vocabulary_terms (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    term TEXT NOT NULL,
    context TEXT NOT NULL DEFAULT '',
    UNIQUE(term, context)
);
CREATE TABLE vocabulary_aliases (
    term_id INTEGER NOT NULL REFERENCES vocabulary_terms(id) ON DELETE CASCADE,
    alias TEXT NOT NULL,
    UNIQUE(term_id, alias)
);
CREATE TABLE vocabulary_candidates (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    history_id INTEGER NOT NULL,
    term TEXT NOT NULL,
    replacement_from TEXT NOT NULL DEFAULT '',
    context TEXT NOT NULL DEFAULT '',
    source TEXT NOT NULL,
    occurrences INTEGER NOT NULL DEFAULT 1,
    status INTEGER NOT NULL DEFAULT 0,
    UNIQUE(history_id, term, replacement_from, context, source)
);
CREATE INDEX vocabulary_candidates_history ON vocabulary_candidates(history_id);
CREATE INDEX vocabulary_candidates_review ON vocabulary_candidates(status, term, replacement_from, context);
CREATE TABLE vocabulary_samples (
    history_id INTEGER PRIMARY KEY,
    corrected_text TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE TRIGGER vocabulary_history_deleted AFTER DELETE ON transcription_history BEGIN
    DELETE FROM vocabulary_candidates WHERE history_id = OLD.id;
    DELETE FROM vocabulary_samples WHERE history_id = OLD.id;
END;
CREATE TRIGGER vocabulary_history_replaced AFTER UPDATE OF transcription_text, post_processed_text ON transcription_history
WHEN OLD.transcription_text IS NOT NEW.transcription_text OR OLD.post_processed_text IS NOT NEW.post_processed_text BEGIN
    DELETE FROM vocabulary_candidates WHERE history_id = OLD.id;
    DELETE FROM vocabulary_samples WHERE history_id = OLD.id;
END;
"#;

#[derive(Debug)]
pub struct Candidate {
    pub id: i64,
    pub proposal: Proposal,
}
#[derive(Debug)]
pub struct Term {
    pub id: i64,
    pub rule: Rule,
}
#[derive(Debug, Default)]
pub struct View {
    pub candidates: Vec<Candidate>,
    pub entries: Vec<Term>,
}
pub type Result<T> = std::result::Result<T, &'static str>;

fn storage(_: rusqlite::Error) -> &'static str {
    "vocabulary_storage_unavailable"
}
fn option(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

pub fn terms(conn: &Connection) -> Result<Vec<Term>> {
    let mut statement = conn
        .prepare("SELECT id, term, context FROM vocabulary_terms ORDER BY id")
        .map_err(storage)?;
    let mut alias_query = conn
        .prepare("SELECT alias FROM vocabulary_aliases WHERE term_id=?1 ORDER BY alias")
        .map_err(storage)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(storage)?;
    let mut out = Vec::new();
    for row in rows {
        let (id, term, context) = row.map_err(storage)?;
        let aliases = alias_query
            .query_map([id], |row| row.get(0))
            .map_err(storage)?
            .collect::<std::result::Result<Vec<String>, _>>()
            .map_err(storage)?;
        out.push(Term {
            id,
            rule: Rule {
                term,
                aliases,
                context: option(context),
            },
        });
    }
    Ok(out)
}

pub fn view(conn: &Connection) -> Result<View> {
    let transaction = conn.unchecked_transaction().map_err(storage)?;
    let candidates = {
        let mut statement = transaction.prepare("SELECT MIN(id), term, replacement_from, context, source, SUM(occurrences) FROM vocabulary_candidates WHERE status=0 GROUP BY term, replacement_from, context, source ORDER BY SUM(occurrences) DESC, MIN(id) LIMIT 200").map_err(storage)?;
        let rows = statement
            .query_map([], |row| {
                Ok(Candidate {
                    id: row.get(0)?,
                    proposal: Proposal {
                        term: row.get(1)?,
                        replacement_from: option(row.get(2)?),
                        context: option(row.get(3)?),
                        source: row.get(4)?,
                        occurrences: row.get::<_, i64>(5)?.clamp(0, i64::from(u32::MAX)) as u32,
                    },
                })
            })
            .map_err(storage)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(storage)?
    };
    let entries = terms(&transaction)?;
    transaction.commit().map_err(storage)?;
    Ok(View {
        candidates,
        entries,
    })
}

fn require_history(conn: &Connection, history_id: i64) -> Result<()> {
    let exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM transcription_history WHERE id=?1)",
            [history_id],
            |row| row.get(0),
        )
        .map_err(storage)?;
    if exists {
        Ok(())
    } else {
        Err("vocabulary_history_missing")
    }
}

fn insert_proposals(conn: &Connection, history_id: i64, proposals: &[Proposal]) -> Result<()> {
    for proposal in proposals.iter().take(40) {
        let alias = proposal.replacement_from.as_deref().unwrap_or("");
        let context = proposal.context.as_deref().unwrap_or("");
        let confirmed: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM vocabulary_terms t WHERE t.term=?1 AND t.context=?2 AND (?3='' OR EXISTS(SELECT 1 FROM vocabulary_aliases a WHERE a.term_id=t.id AND a.alias=?3)))", params![proposal.term, context, alias], |row| row.get(0)).map_err(storage)?;
        if !confirmed {
            conn.execute("INSERT INTO vocabulary_candidates(history_id,term,replacement_from,context,source,occurrences) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(history_id,term,replacement_from,context,source) DO NOTHING", params![history_id, proposal.term, alias, context, proposal.source, proposal.occurrences.max(1)]).map_err(storage)?;
        }
    }
    Ok(())
}

pub fn observe(conn: &mut Connection, history_id: i64, proposals: &[Proposal]) -> Result<()> {
    let transaction = conn.transaction().map_err(storage)?;
    require_history(&transaction, history_id)?;
    insert_proposals(&transaction, history_id, proposals)?;
    transaction.commit().map_err(storage)
}

pub fn review(conn: &mut Connection, id: i64, accept: bool) -> Result<()> {
    let transaction = conn.transaction().map_err(storage)?;
    let candidate: Option<(String, String, String)> = transaction.query_row("SELECT term,replacement_from,context FROM vocabulary_candidates WHERE id=?1 AND status=0", [id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).optional().map_err(storage)?;
    let (term, alias, context) = candidate.ok_or("vocabulary_candidate_missing")?;
    if accept {
        let count: i64 = transaction
            .query_row("SELECT COUNT(*) FROM vocabulary_terms", [], |row| {
                row.get(0)
            })
            .map_err(storage)?;
        let existing: Option<i64> = transaction
            .query_row(
                "SELECT id FROM vocabulary_terms WHERE term=?1 AND context=?2",
                params![term, context],
                |row| row.get(0),
            )
            .optional()
            .map_err(storage)?;
        if existing.is_none() && count >= 500 {
            return Err("vocabulary_term_limit");
        }
        transaction.execute("INSERT INTO vocabulary_terms(term,context) VALUES(?1,?2) ON CONFLICT(term,context) DO NOTHING", params![term, context]).map_err(storage)?;
        if !alias.is_empty() {
            transaction.execute("INSERT OR IGNORE INTO vocabulary_aliases(term_id,alias) SELECT id,?3 FROM vocabulary_terms WHERE term=?1 AND context=?2", params![term, context, alias]).map_err(storage)?;
        }
    }
    transaction.execute("UPDATE vocabulary_candidates SET status=?4 WHERE term=?1 AND replacement_from=?2 AND context=?3 AND status=0", params![term, alias, context, if accept { 1 } else { -1 }]).map_err(storage)?;
    transaction.commit().map_err(storage)
}

pub fn delete_term(conn: &mut Connection, id: i64) -> Result<()> {
    let transaction = conn.transaction().map_err(storage)?;
    transaction
        .execute("DELETE FROM vocabulary_aliases WHERE term_id=?1", [id])
        .map_err(storage)?;
    if transaction
        .execute("DELETE FROM vocabulary_terms WHERE id=?1", [id])
        .map_err(storage)?
        == 0
    {
        return Err("vocabulary_term_missing");
    }
    transaction.commit().map_err(storage)
}

pub fn submit(conn: &mut Connection, history_id: i64, corrected: &str, now: i64) -> Result<()> {
    if corrected.trim().is_empty() || corrected.chars().count() > super::core::MAX_SAMPLE_CHARS {
        return Err("vocabulary_sample_invalid");
    }
    let transaction = conn.transaction().map_err(storage)?;
    let entry: Option<(String, Option<String>)> = transaction
        .query_row(
            "SELECT transcription_text,post_processed_text FROM transcription_history WHERE id=?1",
            [history_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(storage)?;
    let (raw, processed) = entry.ok_or("vocabulary_history_missing")?;
    let actual = processed
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(&raw);
    let proposals = super::core::correction_candidates(&raw, actual, corrected);
    transaction.execute("INSERT INTO vocabulary_samples(history_id,corrected_text,created_at) VALUES(?1,?2,?3) ON CONFLICT(history_id) DO UPDATE SET corrected_text=excluded.corrected_text,created_at=excluded.created_at", params![history_id, corrected, now]).map_err(storage)?;
    transaction.execute("DELETE FROM vocabulary_candidates WHERE history_id=?1 AND source IN ('correction','asr_correction','cleanup_correction')", [history_id]).map_err(storage)?;
    insert_proposals(&transaction, history_id, &proposals)?;
    transaction.commit().map_err(storage)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE transcription_history (id INTEGER PRIMARY KEY, transcription_text TEXT NOT NULL, post_processed_text TEXT, saved BOOLEAN DEFAULT 0); INSERT INTO transcription_history VALUES (1,'在终端打开克劳德 Code',NULL,0),(2,'使用Claude Code',NULL,1);").unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn
    }
    fn proposal() -> Proposal {
        Proposal {
            term: "Claude".into(),
            replacement_from: Some("克劳德".into()),
            context: Some("终端".into()),
            source: "asr_correction".into(),
            occurrences: 1,
        }
    }
    #[test]
    fn confirmation_is_explicit_and_counts_are_idempotent() {
        let mut conn = connection();
        observe(&mut conn, 1, &[proposal()]).unwrap();
        observe(&mut conn, 1, &[proposal()]).unwrap();
        observe(&mut conn, 2, &[proposal()]).unwrap();
        let pending = view(&conn).unwrap();
        assert!(pending.entries.is_empty());
        assert_eq!(pending.candidates.len(), 1);
        assert_eq!(pending.candidates[0].proposal.occurrences, 2);
        review(&mut conn, pending.candidates[0].id, true).unwrap();
        let confirmed = view(&conn).unwrap();
        assert!(confirmed.candidates.is_empty());
        assert_eq!(confirmed.entries[0].rule.aliases, vec!["克劳德"]);
        assert_eq!(confirmed.entries[0].rule.context.as_deref(), Some("终端"));
    }
    #[test]
    fn missing_rows_fail_without_creating_samples() {
        let mut conn = connection();
        assert_eq!(
            observe(&mut conn, 99, &[proposal()]),
            Err("vocabulary_history_missing")
        );
        assert_eq!(
            submit(&mut conn, 99, "Claude", 1),
            Err("vocabulary_history_missing")
        );
        assert_eq!(
            review(&mut conn, 99, true),
            Err("vocabulary_candidate_missing")
        );
        assert_eq!(delete_term(&mut conn, 99), Err("vocabulary_term_missing"));
    }
    #[test]
    fn rejected_proposal_does_not_reappear_on_same_observation() {
        let mut conn = connection();
        observe(&mut conn, 1, &[proposal()]).unwrap();
        let id = view(&conn).unwrap().candidates[0].id;
        review(&mut conn, id, false).unwrap();
        observe(&mut conn, 1, &[proposal()]).unwrap();
        assert!(view(&conn).unwrap().candidates.is_empty());
        assert!(view(&conn).unwrap().entries.is_empty());
    }
    #[test]
    fn history_deletion_clears_samples_candidates_but_keeps_confirmed_terms() {
        let mut conn = connection();
        observe(&mut conn, 2, &[proposal()]).unwrap();
        let id = view(&conn).unwrap().candidates[0].id;
        review(&mut conn, id, true).unwrap();
        submit(&mut conn, 1, "在终端打开Claude Code", 123).unwrap();
        conn.execute("DELETE FROM transcription_history WHERE id=1", [])
            .unwrap();
        assert!(view(&conn).unwrap().candidates.is_empty());
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM vocabulary_samples", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        let confirmed = view(&conn).unwrap();
        assert_eq!(confirmed.entries.len(), 1);
        delete_term(&mut conn, confirmed.entries[0].id).unwrap();
        assert!(view(&conn).unwrap().entries.is_empty());
    }
    #[test]
    fn resubmitting_replaces_sample_without_changing_history() {
        let mut conn = connection();
        submit(&mut conn, 1, "在终端打开Claude Code", 123).unwrap();
        submit(&mut conn, 1, "在终端打开Codex", 124).unwrap();
        let sample: String = conn
            .query_row(
                "SELECT corrected_text FROM vocabulary_samples WHERE history_id=1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(sample, "在终端打开Codex");
        let original: String = conn
            .query_row(
                "SELECT transcription_text FROM transcription_history WHERE id=1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(original, "在终端打开克劳德 Code");
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM vocabulary_samples", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
    #[test]
    fn history_retry_clears_dependent_sample() {
        let mut conn = connection();
        submit(&mut conn, 1, "在终端打开Claude Code", 123).unwrap();
        conn.execute(
            "UPDATE transcription_history SET transcription_text='重试结果' WHERE id=1",
            [],
        )
        .unwrap();
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM vocabulary_samples", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert!(view(&conn).unwrap().candidates.is_empty());
    }
    #[test]
    fn excessive_or_empty_sample_is_rejected() {
        let mut conn = connection();
        assert_eq!(
            submit(&mut conn, 1, " ", 1),
            Err("vocabulary_sample_invalid")
        );
        assert_eq!(
            submit(&mut conn, 1, &"中".repeat(8001), 1),
            Err("vocabulary_sample_invalid")
        );
    }

    #[test]
    fn term_limit_rolls_back_review_without_losing_candidate() {
        let mut conn = connection();
        for i in 0..500 {
            conn.execute(
                "INSERT INTO vocabulary_terms(term) VALUES(?1)",
                [format!("Term{i}")],
            )
            .unwrap();
        }
        observe(&mut conn, 1, &[proposal()]).unwrap();
        let id = view(&conn).unwrap().candidates[0].id;
        assert_eq!(review(&mut conn, id, true), Err("vocabulary_term_limit"));
        assert_eq!(view(&conn).unwrap().candidates[0].id, id);
        assert_eq!(view(&conn).unwrap().entries.len(), 500);
    }

    #[test]
    fn deleting_one_source_reduces_occurrences_without_orphans() {
        let mut conn = connection();
        observe(&mut conn, 1, &[proposal()]).unwrap();
        observe(&mut conn, 2, &[proposal()]).unwrap();
        conn.execute("DELETE FROM transcription_history WHERE id=1", [])
            .unwrap();
        let pending = view(&conn).unwrap();
        assert_eq!(pending.candidates.len(), 1);
        assert_eq!(pending.candidates[0].proposal.occurrences, 1);
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM transcription_history WHERE id=2 AND saved=1",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
    }
}
