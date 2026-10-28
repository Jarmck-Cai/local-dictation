//! Synthetic histories only; checks exercise the production SQL and extractor.
use super::{collected_core, collected_store as collected, core, store};
use rusqlite::{params, Connection};

fn db() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE transcription_history(id INTEGER PRIMARY KEY, timestamp INTEGER NOT NULL, transcription_text TEXT NOT NULL, post_processed_text TEXT);").unwrap();
    db.execute_batch(store::SCHEMA).unwrap();
    db.execute_batch(store::MANAGEMENT_SCHEMA).unwrap();
    db.execute(
        "INSERT INTO vocabulary_terms(id,term) VALUES(73,'Confirmed')",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO vocabulary_aliases(term_id,alias) VALUES(73,'Confirmd')",
        [],
    )
    .unwrap();
    db.execute_batch(collected::SCHEMA).unwrap();
    db
}
fn history(db: &Connection, id: i64, raw: &str) {
    db.execute(
        "INSERT INTO transcription_history VALUES(?1,?1,?2,'AIOnlyFinal')",
        params![id, raw],
    )
    .unwrap();
}
fn page(db: &Connection, filter: &str) -> collected::Page {
    collected::page(db, None, Some(filter), None, None).unwrap()
}
fn finish(db: &mut Connection) {
    while collected::backfill_batch(db, 16).unwrap() != 0 {}
}

#[test]
fn distinct_histories_threshold_and_replay() {
    let mut db = db();
    history(&db, 1, "Rust Rust Rust");
    collected::observe_history(&mut db, 1).unwrap();
    collected::observe_history(&mut db, 1).unwrap();
    assert!(page(&db, "all").entries.is_empty());
    history(&db, 2, "rust");
    finish(&mut db);
    finish(&mut db);
    let result = page(&db, "all");
    assert_eq!(result.total, 1);
    assert_eq!(result.entries[0].term, "Rust");
    assert_eq!(result.entries[0].record_count, 2);
    assert_eq!(result.entries[0].last_seen, Some(2));
}

#[test]
fn liked_and_deleted_labels_survive_history_expiry_and_backfill() {
    let mut db = db();
    history(&db, 1, "Rust SQLite");
    history(&db, 2, "Rust SQLite");
    finish(&mut db);
    let entries = page(&db, "all").entries;
    let rust = entries.iter().find(|t| t.term == "Rust").unwrap().id;
    let sqlite = entries.iter().find(|t| t.term == "SQLite").unwrap().id;
    collected::set_liked(&mut db, rust, true).unwrap();
    collected::set_deleted(&mut db, sqlite, true).unwrap();
    assert_eq!(page(&db, "all").total, 1);
    assert_eq!(page(&db, "liked").entries[0].id, rust);
    db.execute("DELETE FROM transcription_history", []).unwrap();
    assert_eq!(page(&db, "liked").entries[0].record_count, 0);
    assert_eq!(page(&db, "deleted").entries[0].id, sqlite);
    history(&db, 3, "SQLite");
    history(&db, 4, "SQLite");
    finish(&mut db);
    assert_eq!(
        page(&db, "all").total,
        1,
        "background observation cannot restore deletion"
    );
    collected::set_deleted(&mut db, sqlite, false).unwrap();
    assert_eq!(page(&db, "all").total, 2);
    collected::set_liked(&mut db, rust, false).unwrap();
    assert!(page(&db, "liked").entries.is_empty());
    assert_eq!(page(&db, "all").total, 1);
    collected::set_liked(&mut db, sqlite, true).unwrap();
    collected::set_liked(&mut db, sqlite, false).unwrap();
    assert!(page(&db, "liked").entries.is_empty());
}

#[test]
fn history_replacement_clears_old_observations_and_only_raw_is_a_source() {
    let mut db = db();
    history(&db, 1, "Rust");
    history(&db, 2, "Rust");
    finish(&mut db);
    let id = page(&db, "all").entries[0].id;
    collected::set_liked(&mut db, id, true).unwrap();
    db.execute(
        "UPDATE transcription_history SET post_processed_text='FinalOther' WHERE id=1",
        [],
    )
    .unwrap();
    assert_eq!(page(&db, "all").entries[0].record_count, 2);
    db.execute(
        "UPDATE transcription_history SET transcription_text='SQLite' WHERE id=1",
        [],
    )
    .unwrap();
    assert_eq!(page(&db, "all").entries[0].record_count, 1);
    collected::observe_history(&mut db, 1).unwrap();
    db.execute("DELETE FROM transcription_history WHERE id=2", [])
        .unwrap();
    assert_eq!(page(&db, "all").entries[0].record_count, 0);
    assert!(
        collected::observe_history(&mut db, 2).is_ok(),
        "late observer cannot resurrect expired row"
    );
    history(&db, 3, "SQLite");
    finish(&mut db);
    assert_eq!(page(&db, "all").total, 2);
    assert!(!page(&db, "all")
        .entries
        .iter()
        .any(|t| t.term.contains("Final")));
    let rules: Vec<_> = store::active_terms(&db)
        .unwrap()
        .into_iter()
        .map(|t| t.rule)
        .collect();
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].id, 73);
    assert_eq!(
        core::apply_rules("Rust SQLite Confirmd", &rules),
        "Rust SQLite Confirmed"
    );
    assert!(core::relevant_terms("Rust SQLite", &rules).is_empty());
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM vocabulary_candidates", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn search_filters_pagination_and_bounds_are_literal() {
    let mut db = db();
    for id in 1..=2 {
        history(&db, id, "Type_Script TypeScript C++ C# Rust");
    }
    finish(&mut db);
    assert_eq!(
        collected::page(&db, Some("_"), None, None, None)
            .unwrap()
            .total,
        1
    );
    assert_eq!(
        collected::page(&db, Some("%"), None, None, None)
            .unwrap()
            .total,
        0
    );
    assert_eq!(
        collected::page(&db, Some("c++"), None, None, None)
            .unwrap()
            .total,
        1
    );
    assert_eq!(
        collected::page(&db, Some("' OR 1=1 --"), None, None, None)
            .unwrap()
            .total,
        0
    );
    let first = collected::page(&db, None, None, Some(0), Some(2)).unwrap();
    let second = collected::page(&db, None, None, Some(2), Some(2)).unwrap();
    assert_eq!(first.entries.len(), 2);
    assert_eq!(second.entries.len(), 2);
    assert_ne!(first.entries[0].id, second.entries[0].id);
    assert_eq!(
        collected::page(&db, None, None, None, Some(999))
            .unwrap()
            .limit,
        100
    );
    assert_eq!(
        collected::page(&db, None, None, None, Some(0))
            .unwrap()
            .limit,
        1
    );
    assert!(collected::page(&db, Some(&"中".repeat(257)), None, None, None).is_err());
    assert!(collected::page(&db, None, Some("invalid"), None, None).is_err());
    assert!(collected::set_deleted(&mut db, 999, true).is_err());
}

#[test]
fn ordinary_terms_disappear_at_expiry_and_likes_override_threshold() {
    let mut db = db();
    history(&db, 1, "Rust SQLite");
    history(&db, 2, "Rust");
    finish(&mut db);
    let rust = page(&db, "all").entries[0].id;
    let sqlite = db
        .query_row(
            "SELECT id FROM collected_terms WHERE term='SQLite'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    collected::set_liked(&mut db, sqlite, true).unwrap();
    assert_eq!(page(&db, "all").total, 2);
    db.execute("DELETE FROM transcription_history WHERE id=1", [])
        .unwrap();
    assert_eq!(page(&db, "all").entries[0].id, sqlite);
    db.execute("DELETE FROM transcription_history", []).unwrap();
    assert_eq!(page(&db, "all").total, 1);
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM collected_terms WHERE id=?1",
            [rust],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn stale_snapshot_cannot_repopulate_after_delete_or_replacement() {
    let mut db = db();
    history(&db, 1, "OldWord");
    history(&db, 2, "OldWord");
    finish(&mut db);
    db.execute(
        "UPDATE transcription_history SET transcription_text='NewWord' WHERE id=1",
        [],
    )
    .unwrap();
    assert!(!collected::observe_snapshot(&mut db, 1, "OldWord").unwrap());
    db.execute("DELETE FROM transcription_history WHERE id=2", [])
        .unwrap();
    assert!(!collected::observe_snapshot(&mut db, 2, "OldWord").unwrap());
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM collected_term_observations",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    finish(&mut db);
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM collected_term_observations",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

#[test]
fn basic_english_terms_keep_distinctions_and_skip_paths_urls_code() {
    let terms = collected_core::extract("Rust rust SQLite C++ C# Type_Script https://Example.com/a C:/Projects/Code src/main.rs foo::bar() let value = BadCode; Rust，SQLite");
    for expected in ["Rust", "SQLite", "C++", "C#", "Type_Script"] {
        assert!(terms.iter().any(|t| t == expected), "missing {expected}");
    }
    assert_eq!(
        terms
            .iter()
            .filter(|t| t.eq_ignore_ascii_case("rust"))
            .count(),
        1
    );
    for forbidden in [
        "Example.com",
        "Code",
        "src/main.rs",
        "foo",
        "bar",
        "BadCode",
    ] {
        assert!(
            !terms.iter().any(|t| t == forbidden),
            "code/path leak: {forbidden}"
        );
    }
}

#[test]
fn punctuation_does_not_hide_english_terms() {
    let terms = collected_core::extract("Use Rust. Claude Code, Codex and API!");
    for expected in ["Rust", "Claude", "Code", "Codex", "API"] {
        assert!(terms.iter().any(|t| t == expected), "missing {expected}");
    }
}

#[test]
fn chinese_dictionary_segmentation_does_not_emit_character_shingles() {
    let terms = collected_core::extract("今天讨论数据库设计和语音识别，使用本地模型整理项目需求。");
    assert!(terms.iter().any(|t| t == "数据库"));
    assert!(terms.iter().any(|t| t == "语音" || t == "语音识别"));
    assert!(!terms.iter().any(|t| t == "据库" || t == "库设"));
    assert!(terms.iter().all(|t| t.chars().count() >= 2));
}

#[test]
fn minimum_chinese_coverage_is_counted_across_distinct_records() {
    let mut db = db();
    history(&db, 1, "在微信里检查数据库连接和快捷键。");
    history(&db, 2, "微信的数据库配置要保留，快捷键不要修改。");
    finish(&mut db);
    for expected in ["微信", "数据库", "快捷键"] {
        let entry = page(&db, "all")
            .entries
            .into_iter()
            .find(|t| t.term == expected)
            .unwrap_or_else(|| {
                panic!(
                    "missing {expected}: {:?}",
                    collected_core::extract("在微信里检查数据库连接和快捷键。")
                )
            });
        assert_eq!(entry.record_count, 2);
    }
}

#[test]
fn schema_seven_upgrade_preserves_confirmed_ids_and_is_idempotent() {
    use rusqlite_migration::{Migrations, M};
    let migrations=Migrations::new(vec![
        M::up("CREATE TABLE transcription_history(id INTEGER PRIMARY KEY AUTOINCREMENT,file_name TEXT NOT NULL,timestamp INTEGER NOT NULL,saved BOOLEAN NOT NULL DEFAULT 0,title TEXT NOT NULL,transcription_text TEXT NOT NULL);"),
        M::up("ALTER TABLE transcription_history ADD COLUMN post_processed_text TEXT;"),
        M::up("ALTER TABLE transcription_history ADD COLUMN post_process_prompt TEXT;"),
        M::up("ALTER TABLE transcription_history ADD COLUMN post_process_requested BOOLEAN NOT NULL DEFAULT 0;"),
        M::up(store::SCHEMA), M::up(store::MANAGEMENT_SCHEMA), M::up(collected::SCHEMA),
    ]);
    migrations.validate().unwrap();
    for upgrade in [false, true] {
        let mut db = Connection::open_in_memory().unwrap();
        if upgrade {
            migrations.to_version(&mut db, 6).unwrap();
            db.execute(
                "INSERT INTO vocabulary_terms(id,term,enabled) VALUES(73,'Confirmed',0)",
                [],
            )
            .unwrap();
            db.execute(
                "INSERT INTO vocabulary_aliases(term_id,alias) VALUES(73,'Confirmd')",
                [],
            )
            .unwrap();
        }
        migrations.to_latest(&mut db).unwrap();
        migrations.to_latest(&mut db).unwrap();
        assert_eq!(
            db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            7
        );
        if upgrade {
            assert_eq!(store::terms(&db).unwrap()[0].id, 73);
            assert!(!store::terms(&db).unwrap()[0].enabled);
            assert_eq!(store::terms(&db).unwrap()[0].rule.aliases, vec!["Confirmd"]);
        }
        assert_eq!(page(&db, "all").total, 0);
    }
}

#[test]
fn explicit_correction_is_not_hidden_by_old_transcript_candidate_limit() {
    let db = db();
    for index in 0..201 {
        db.execute("INSERT INTO vocabulary_candidates(history_id,term,source,occurrences) VALUES(1,?1,'transcript',500)",[format!("OldAuto{index}")]).unwrap();
    }
    db.execute("INSERT INTO vocabulary_candidates(history_id,term,replacement_from,source) VALUES(1,'FixedOnly','FixedTypo','asr_correction')",[]).unwrap();
    let view = store::reviewable_view(&db).unwrap();
    assert_eq!(view.candidates.len(), 1);
    assert_eq!(view.candidates[0].proposal.term, "FixedOnly");
    assert_eq!(view.entries[0].id, 73);
}

#[test]
fn locative_context_preserves_known_place_names() {
    for name in ["马德里", "新德里"] {
        let terms = collected_core::extract(&format!("我在{name}工作。{name}的数据库项目。"));
        assert!(terms.iter().any(|t| t == name), "lost {name}");
        assert!(
            !terms.iter().any(|t| t == name.strip_suffix('里').unwrap()),
            "truncated known name {name}"
        );
    }
}

#[test]
fn declarations_do_not_collect_code_variables_as_prose_terms() {
    let terms = collected_core::extract("Rust discussion; let BadCode = OtherWord; SQLite");
    assert!(terms.iter().any(|t| t == "Rust"));
    assert!(terms.iter().any(|t| t == "SQLite"));
    assert!(!terms.iter().any(|t| t == "BadCode" || t == "OtherWord"));
}
