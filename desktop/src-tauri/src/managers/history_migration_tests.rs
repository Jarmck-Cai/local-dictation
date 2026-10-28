use super::*;
#[test]
fn actual_v6_upgrade_preserves_history_and_confirmed_terms() {
    let mut connection = Connection::open_in_memory().unwrap();
    Migrations::new(MIGRATIONS[..6].to_vec())
        .to_latest(&mut connection)
        .unwrap();
    connection.execute_batch(
        "INSERT INTO transcription_history(id,file_name,timestamp,saved,title,transcription_text)
         VALUES(71,'synthetic.wav',123,0,'','synthetic Codex');
         INSERT INTO vocabulary_terms(id,term,context,enabled) VALUES(42,'Codex','',1);
         INSERT INTO vocabulary_aliases(term_id,alias) VALUES(42,'code ex');"
    ).unwrap();
    Migrations::new(MIGRATIONS.to_vec())
        .to_latest(&mut connection)
        .unwrap();
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(
        version, 7,
        "collected terms must be appended as migration seven"
    );
    let old: (String,String,bool) = connection.query_row(
        "SELECT h.transcription_text,t.term,t.enabled FROM transcription_history h CROSS JOIN vocabulary_terms t WHERE h.id=71 AND t.id=42",
        [],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(
        old,
        ("synthetic Codex".to_owned(), "Codex".to_owned(), true)
    );
    let alias: String = connection
        .query_row(
            "SELECT alias FROM vocabulary_aliases WHERE term_id=42",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(alias, "code ex");
    connection
        .execute_batch(
            "INSERT INTO collected_terms(id,normalized,term,liked) VALUES(9,'codex','Codex',1);
         INSERT INTO collected_term_observations(history_id,term_id,timestamp) VALUES(71,9,123);
         INSERT INTO collected_history_state(history_id) VALUES(71);
         DELETE FROM transcription_history WHERE id=71;",
        )
        .unwrap();
    let observations: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM collected_term_observations",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let liked: bool = connection
        .query_row("SELECT liked FROM collected_terms WHERE id=9", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(observations, 0);
    assert!(liked);
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM vocabulary_terms WHERE id=42",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    Migrations::new(MIGRATIONS.to_vec())
        .to_latest(&mut connection)
        .unwrap();
}
