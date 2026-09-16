use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use graphmem::Database;

fn test_database() -> (Database, PathBuf) {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is valid")
        .as_nanos();
    let path = std::env::temp_dir()
        .join(format!(
            "graphmem-search-test-{}-{nonce}",
            std::process::id()
        ))
        .join("memory.sqlite");
    let database = Database::open(&path).expect("database opens");
    (database, path)
}

fn remove_database(path: &Path) {
    if let Some(parent) = path.parent() {
        fs::remove_dir_all(parent).expect("test database is removed");
    }
}

#[test]
fn fts_tracks_memory_insert_update_and_delete() {
    let (database, path) = test_database();
    let memory = database
        .create_memory("retry queue convention", "fact", 0.0)
        .expect("memory is created");
    assert!(memory.id > 0);
    assert_eq!(
        database.search_memories("retry", None, 10).unwrap().len(),
        1
    );

    database
        .update_memory(memory.id, "sqlite index convention", "fact", 0.0)
        .expect("memory is updated");
    assert!(
        database
            .search_memories("retry", None, 10)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        database.search_memories("sqlite", None, 10).unwrap().len(),
        1
    );

    assert!(database.delete_memory(memory.id).unwrap());
    assert!(
        database
            .search_memories("sqlite", None, 10)
            .unwrap()
            .is_empty()
    );
    let replacement = database
        .create_memory("replacement", "fact", 0.0)
        .expect("replacement memory is created");
    assert!(replacement.id > memory.id);
    drop(database);
    remove_database(&path);
}

#[test]
fn supports_literal_fts_operators() {
    let (database, path) = test_database();
    database
        .create_memory("cargo nextest integration", "fact", 0.0)
        .expect("cargo memory is created");
    database
        .create_memory("sqlite migration", "fact", 0.0)
        .expect("sqlite memory is created");

    assert_eq!(
        database
            .search_memories("\"cargo nextest\"", None, 10)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        database
            .search_memories("cargo OR sqlite", None, 10)
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        database
            .search_memories("cargo NOT sqlite", None, 10)
            .unwrap()
            .len(),
        1
    );
    drop(database);
    remove_database(&path);
}

#[test]
fn plain_language_queries_match_any_term_and_rank_by_overlap() {
    let (database, path) = test_database();
    let both = database
        .create_memory("Scott Derrickson is an American film director", "fact", 0.0)
        .expect("memory is created");
    let one = database
        .create_memory("Ed Wood was an American filmmaker", "fact", 0.0)
        .expect("memory is created");
    database
        .create_memory("sqlite migration notes", "fact", 0.0)
        .expect("memory is created");

    let results = database
        .search_memories(
            "Were Scott Derrickson and Ed Wood, directors, American?",
            None,
            10,
        )
        .expect("a natural-language question does not error");
    let ids = results
        .iter()
        .map(|result| result.memory.id)
        .collect::<Vec<_>>();
    assert_eq!(ids.len(), 2, "no single memory contains every word");
    assert!(ids.contains(&both.id) && ids.contains(&one.id));

    let results = database
        .search_memories("scott derrickson director", None, 10)
        .unwrap();
    assert_eq!(
        results[0].memory.id, both.id,
        "more matching terms rank first"
    );

    let results = database
        .search_memories(
            "Who directed \"Ed Wood\" (Film) and film* in 199?",
            None,
            10,
        )
        .expect("quotes, parentheses, and prefixes in a question do not error");
    assert_eq!(
        results.len(),
        2,
        "the phrase and the film* prefix each match"
    );
    drop(database);
    remove_database(&path);
}

#[test]
fn explicit_and_keeps_requiring_every_term() {
    let (database, path) = test_database();
    database
        .create_memory("cargo nextest integration", "fact", 0.0)
        .expect("memory is created");
    database
        .create_memory("cargo build notes", "fact", 0.0)
        .expect("memory is created");
    assert_eq!(
        database
            .search_memories("cargo AND nextest", None, 10)
            .unwrap()
            .len(),
        1
    );
    drop(database);
    remove_database(&path);
}

#[test]
fn hyphenated_query_terms_do_not_error() {
    let (database, path) = test_database();
    database
        .create_memory("smoke test entry", "fact", 0.0)
        .expect("memory is created");
    let results = database
        .search_memories("smoke-test", None, 10)
        .expect("hyphenated query does not error");
    assert_eq!(results.len(), 1);
    drop(database);
    remove_database(&path);
}

#[test]
fn hyphenated_terms_keep_surrounding_operator_semantics() {
    let (database, path) = test_database();
    let matching = database
        .create_memory("checkout flow incident happened", "fact", 0.0)
        .expect("memory is created");
    database
        .create_memory("unrelated deployment note", "fact", 0.0)
        .expect("memory is created");
    let results = database
        .search_memories("incident OR checkout-latency", None, 10)
        .expect("hyphenated operand does not error");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].memory.id, matching.id);
    drop(database);
    remove_database(&path);
}

#[test]
fn filters_search_results_by_exact_scope() {
    let (mut database, path) = test_database();
    let first_scope = database.create_scope("repo:first").unwrap();
    let second_scope = database.create_scope("repo:second").unwrap();
    let first = database
        .create_memory("shared deployment rule", "fact", 0.0)
        .unwrap();
    let second = database
        .create_memory("shared deployment rule", "fact", 0.0)
        .unwrap();
    database.attach_scopes(first.id, &[first_scope.id]).unwrap();
    database
        .attach_scopes(second.id, &[second_scope.id])
        .unwrap();

    let results = database
        .search_memories("deployment", Some(" repo:first "), 10)
        .unwrap();
    assert_eq!(
        results
            .iter()
            .map(|result| result.memory.id)
            .collect::<Vec<_>>(),
        [first.id]
    );
    drop(database);
    remove_database(&path);
}

#[test]
fn ranks_importance_then_recency_after_bm25() {
    let (database, path) = test_database();
    let low = database
        .create_memory("retry worker policy", "fact", 0.1)
        .unwrap();
    let high = database
        .create_memory("retry worker policy", "fact", 0.9)
        .unwrap();
    database
        .create_memory("sqlite migration policy", "fact", 0.0)
        .unwrap();
    let results = database.search_memories("retry", None, 10).unwrap();
    assert_eq!(results[0].memory.id, high.id);
    assert_eq!(results[1].memory.id, low.id);
    assert!(results[0].score >= results[1].score);

    let first = database
        .create_memory("sqlite maintenance policy", "fact", 0.0)
        .unwrap();
    thread::sleep(Duration::from_millis(2));
    let second = database
        .create_memory("sqlite maintenance policy", "fact", 0.0)
        .unwrap();
    let results = database.search_memories("maintenance", None, 10).unwrap();
    assert_eq!(results[0].memory.id, second.id);
    assert!(results[0].memory.updated_at > results[1].memory.updated_at);
    assert_ne!(first.id, second.id);
    drop(database);
    remove_database(&path);
}
