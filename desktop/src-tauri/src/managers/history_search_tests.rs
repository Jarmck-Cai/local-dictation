use super::*;

fn database() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE transcription_history (
            id INTEGER PRIMARY KEY, file_name TEXT NOT NULL DEFAULT 'synthetic.wav',
            timestamp INTEGER NOT NULL DEFAULT 0, saved BOOLEAN NOT NULL DEFAULT 0,
            title TEXT NOT NULL DEFAULT '', transcription_text TEXT NOT NULL,
            post_processed_text TEXT, post_process_prompt TEXT,
            post_process_requested BOOLEAN NOT NULL DEFAULT 0
        );",
    )
    .unwrap();
    for id in 1..=350 {
        conn.execute(
            "INSERT INTO transcription_history (id, transcription_text) VALUES (?1, ?2)",
            params![id, format!("synthetic-{id}")],
        )
        .unwrap();
    }
    conn
}

fn search(
    conn: &Connection,
    cursor: Option<i64>,
    limit: Option<usize>,
    query: Option<&str>,
    anchor: Option<i64>,
) -> HistorySearchPage {
    HistoryManager::search_history_entries_with_conn(conn, cursor, limit, query, anchor).unwrap()
}

#[test]
fn pages_cover_all_350_with_a_stable_anchor() {
    let conn = database();
    let first = search(&conn, None, None, None, None);
    assert_eq!(first.entries.len(), 30);
    assert_eq!(first.anchor_id, Some(350));
    let mut ids: Vec<_> = first.entries.iter().map(|e| e.id).collect();
    let mut page = first;
    conn.execute(
        "INSERT INTO transcription_history (id, transcription_text) VALUES (351, 'new')",
        [],
    )
    .unwrap();
    while page.has_more {
        page = search(&conn, page.next_cursor, None, None, page.anchor_id);
        ids.extend(page.entries.iter().map(|e| e.id));
    }
    assert_eq!(ids, (1..=350).rev().collect::<Vec<_>>());
    assert_eq!(search(&conn, None, None, None, None).entries[0].id, 351);
    assert_eq!(page.next_cursor, None);
}

#[test]
fn raw_and_final_are_searchable_beyond_the_prefetch_window() {
    let conn = database();
    conn.execute("UPDATE transcription_history SET transcription_text = '原文专词', post_processed_text = 'final-only' WHERE id = 1", []).unwrap();
    assert_eq!(
        search(&conn, None, None, Some("原文"), None).entries[0].id,
        1
    );
    assert_eq!(
        search(&conn, None, None, Some("final-only"), None).entries[0].id,
        1
    );
    assert_eq!(
        search(&conn, None, None, Some("no match"), None)
            .entries
            .len(),
        0
    );
}

#[test]
fn patterns_and_sql_are_literal_bound_values() {
    let conn = database();
    conn.execute(
        "UPDATE transcription_history SET transcription_text = ?1 WHERE id = 1",
        params!["100%_\\' OR 1=1 -- 中文😀"],
    )
    .unwrap();
    for query in ["%", "_", "\\", "' OR 1=1 --", "中文😀"] {
        let page = search(&conn, None, None, Some(query), None);
        assert_eq!(page.entries.len(), 1, "query should be literal");
        assert_eq!(page.entries[0].id, 1);
    }
}

#[test]
fn limits_unicode_and_empty_queries_are_bounded() {
    let conn = database();
    assert_eq!(search(&conn, None, Some(0), None, None).entries.len(), 1);
    assert_eq!(
        search(&conn, None, Some(999), None, None).entries.len(),
        100
    );
    assert_eq!(
        search(&conn, None, None, Some(" \t\n"), None).entries.len(),
        30
    );
    assert!(HistoryManager::search_history_entries_with_conn(
        &conn,
        None,
        None,
        Some(&"😀".repeat(257)),
        None
    )
    .is_err());
    assert!(HistoryManager::search_history_entries_with_conn(
        &conn,
        None,
        None,
        Some(&"😀".repeat(256)),
        None
    )
    .is_ok());
}

#[test]
fn deletions_and_empty_database_remain_recoverable() {
    let conn = database();
    let page = search(&conn, None, None, None, None);
    conn.execute(
        "DELETE FROM transcription_history WHERE id IN (350, 320)",
        [],
    )
    .unwrap();
    let next = search(&conn, page.next_cursor, None, None, page.anchor_id);
    assert_eq!(next.entries[0].id, 319);
    assert_eq!(next.entries.len(), 30);
    conn.execute("DELETE FROM transcription_history", [])
        .unwrap();
    let empty = search(&conn, None, None, None, None);
    assert!(empty.entries.is_empty());
    assert_eq!(empty.anchor_id, None);
    assert!(!empty.has_more);
}
