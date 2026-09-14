use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

use graphmem::Database;
use rusqlite::Connection;
use uuid::Uuid;

fn test_database() -> (Database, PathBuf) {
    let path = std::env::temp_dir()
        .join(format!("graphmem-search-test-{}", Uuid::now_v7()))
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
fn migrates_v1_memories_and_backfills_fts() {
    let path = std::env::temp_dir()
        .join(format!("graphmem-search-migration-{}", Uuid::now_v7()))
        .join("memory.sqlite");
    fs::create_dir_all(path.parent().unwrap()).expect("migration test directory is created");
    let memory_id = Uuid::now_v7();
    let connection = Connection::open(&path).expect("v1 database opens");
    connection
        .execute_batch(&format!(
            "CREATE TABLE memories (
                 id TEXT PRIMARY KEY NOT NULL,
                 content TEXT NOT NULL,
                 memory_type TEXT NOT NULL,
                 importance REAL NOT NULL DEFAULT 0.0,
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL,
                 last_accessed_at INTEGER,
                 access_count INTEGER NOT NULL DEFAULT 0 CHECK (access_count >= 0)
             );
             CREATE TABLE scopes (
                 id TEXT PRIMARY KEY NOT NULL,
                 name TEXT NOT NULL UNIQUE,
                 created_at INTEGER NOT NULL
             );
             CREATE TABLE memory_scopes (
                 memory_id TEXT NOT NULL,
                 scope_id TEXT NOT NULL,
                 PRIMARY KEY (memory_id, scope_id)
             );
             INSERT INTO memories
                 (id, content, memory_type, importance, created_at, updated_at)
                 VALUES ('{memory_id}', 'legacy retry convention', 'fact', 0.5, 1, 1);
             PRAGMA user_version = 1;"
        ))
        .expect("v1 schema is created");
    drop(connection);

    let database = Database::open(&path).expect("v1 database migrates");
    let results = database
        .search_memories("legacy", None, 10)
        .expect("legacy memory is searchable");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].memory.id, memory_id);
    assert!(results[0].score.is_finite());
    drop(database);
    remove_database(&path);
}

#[test]
fn fts_tracks_memory_insert_update_and_delete() {
    let (database, path) = test_database();
    let memory = database
        .create_memory("retry queue convention", "fact", 0.0)
        .expect("memory is created");
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
