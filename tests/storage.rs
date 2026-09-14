use std::{
    fs,
    path::{Path, PathBuf},
};

use graphmem::Database;
use uuid::Uuid;

fn test_database() -> (Database, PathBuf) {
    let path = std::env::temp_dir()
        .join(format!("graphmem-test-{}", Uuid::now_v7()))
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
fn migrates_reopens_and_supports_memory_scope_crud() {
    let (mut database, path) = test_database();
    let memory = database
        .create_memory("  use nextest  ", "convention", 0.8)
        .expect("memory is created");
    let repository = database
        .create_scope(" repo:/workspace/project ")
        .expect("scope is created");
    let user = database
        .create_scope("user:test")
        .expect("scope is created");

    database
        .attach_scopes(memory.id, &[repository.id, user.id])
        .expect("scopes are attached");
    assert_eq!(database.list_memory_scopes(memory.id).unwrap().len(), 2);
    assert!(
        database
            .update_memory(memory.id, "use cargo nextest", "fact", 1.0)
            .unwrap()
    );
    assert_eq!(
        database.get_memory(memory.id).unwrap().unwrap().memory_type,
        "fact"
    );
    assert!(database.detach_scope(memory.id, user.id).unwrap());
    assert!(database.delete_memory(memory.id).unwrap());
    assert!(database.get_memory(memory.id).unwrap().is_none());

    drop(database);
    let database = Database::open(&path).expect("database reopens");
    assert!(database.get_scope(repository.id).unwrap().is_some());
    drop(database);
    remove_database(&path);
}

#[test]
fn supports_entity_and_edge_crud_with_directional_lookup() {
    let (database, path) = test_database();
    let source = database
        .create_entity("component", "API", "api")
        .expect("source is created");
    let target = database
        .create_entity("library", "SQLite", "sqlite")
        .expect("target is created");
    let edge = database
        .create_edge(
            source.id,
            "depends_on",
            target.id,
            Some("runtime dependency"),
        )
        .expect("edge is created");

    assert_eq!(
        database.list_outgoing_edges(source.id).unwrap(),
        vec![edge.clone()]
    );
    assert_eq!(
        database.list_incoming_edges(target.id).unwrap(),
        vec![edge.clone()]
    );
    assert!(database.update_edge(edge.id, "uses", None).unwrap());
    assert_eq!(
        database.get_edge(edge.id).unwrap().unwrap().relation,
        "uses"
    );
    assert!(database.delete_entity(target.id).unwrap());
    assert!(database.get_edge(edge.id).unwrap().is_none());
    drop(database);
    remove_database(&path);
}

#[test]
fn failed_scope_batch_rolls_back_all_associations() {
    let (mut database, path) = test_database();
    let memory = database
        .create_memory("transactional memory", "fact", 0.0)
        .expect("memory is created");
    let scope = database.create_scope("global").expect("scope is created");

    let result = database.attach_scopes(memory.id, &[scope.id, Uuid::now_v7()]);
    assert!(result.is_err());
    assert!(database.list_memory_scopes(memory.id).unwrap().is_empty());

    drop(database);
    remove_database(&path);
}

#[test]
fn default_database_uses_graphmem_home_override() {
    let root = std::env::temp_dir().join(format!("graphmem-home-{}", Uuid::now_v7()));
    unsafe { std::env::set_var("GRAPHMEM_HOME", &root) };
    let database = Database::open_default().expect("default database opens");
    drop(database);
    assert!(root.join("memory.sqlite").is_file());
    unsafe { std::env::remove_var("GRAPHMEM_HOME") };
    fs::remove_dir_all(root).expect("default database is removed");
}

#[test]
fn rejects_invalid_values_and_duplicate_entities() {
    let (database, path) = test_database();
    assert!(database.create_memory(" ", "fact", 0.0).is_err());
    assert!(database.create_memory("content", "fact", f64::NAN).is_err());
    database
        .create_entity("tool", "Cargo", "cargo")
        .expect("entity is created");
    assert!(database.create_entity("tool", "Cargo 2", "cargo").is_err());
    drop(database);
    remove_database(&path);
}
