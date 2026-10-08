use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use graphmem::{Database, EntityReference, GraphDirection, Relation};

fn test_database() -> (Database, PathBuf) {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is valid")
        .as_nanos();
    let path = std::env::temp_dir()
        .join(format!("graphmem-test-{}-{nonce}", std::process::id()))
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
fn scoped_listing_limits_results_after_filtering() {
    let (mut database, path) = test_database();
    let local_scope = vec!["repo:/local".to_owned()];
    let local = database
        .remember_with_graph("local", "decision", 0.0, &local_scope, &[], &[])
        .unwrap();
    let global = database
        .remember_with_graph(
            "global",
            "observation",
            0.0,
            &["global".to_owned()],
            &[],
            &[],
        )
        .unwrap();
    let legacy = database.create_memory("legacy", "decision", 0.0).unwrap();
    database
        .remember_with_graph(
            "foreign",
            "decision",
            0.0,
            &["repo:/foreign".to_owned()],
            &[],
            &[],
        )
        .unwrap();

    for (limit, expected) in [
        (0, vec![]),
        (1, vec![legacy.id]),
        (2, vec![legacy.id, global.id]),
        (usize::MAX, vec![legacy.id, global.id, local.id]),
    ] {
        let ids = database
            .list_memories_in_scopes_limited(&local_scope, None, limit)
            .unwrap()
            .into_iter()
            .map(|memory| memory.id)
            .collect::<Vec<_>>();
        assert_eq!(ids, expected);
    }
    let decisions = database
        .list_memories_in_scopes_limited(&local_scope, Some("DECISION"), 2)
        .unwrap();
    assert_eq!(
        decisions.iter().map(|memory| memory.id).collect::<Vec<_>>(),
        vec![legacy.id, local.id]
    );
    assert_eq!(
        database
            .list_memories_in_scopes(&local_scope, None)
            .unwrap()
            .len(),
        3
    );
    drop(database);
    remove_database(&path);
}

#[test]
fn checked_flush_rolls_back_when_a_scope_is_rejected() {
    let (mut database, path) = test_database();
    let memory = database
        .remember_with_graph(
            "foreign memory",
            "observation",
            0.0,
            &["global".to_owned(), "repo:/foreign".to_owned()],
            &[EntityReference {
                kind: "component".to_owned(),
                name: "retained".to_owned(),
            }],
            &[],
        )
        .unwrap();
    let rejected = database.flush_with_scope_check(|scope| {
        if scope == "global" {
            Ok(())
        } else {
            Err(graphmem::StorageError::Invalid {
                field: "scope",
                message: "read-only",
            })
        }
    });
    assert!(rejected.is_err());
    assert_eq!(
        database.get_memory(memory.id).unwrap().unwrap().content,
        "foreign memory"
    );
    assert_eq!(database.list_memory_scopes(memory.id).unwrap().len(), 2);
    assert_eq!(database.stats().unwrap().entities, 1);
    database
        .flush_with_scope_check(|_| Ok::<_, graphmem::StorageError>(()))
        .unwrap();
    assert_eq!(database.stats().unwrap().memories, 0);
    drop(database);
    remove_database(&path);
}

#[test]
fn checked_flush_cannot_delete_a_concurrent_foreign_insertion() {
    let (mut database, path) = test_database();
    let local = database
        .remember_with_graph(
            "local",
            "observation",
            0.0,
            &["global".to_owned()],
            &[],
            &[],
        )
        .unwrap();
    let mut concurrent = Database::open(&path).unwrap();
    let mut inserted = None;
    let result = database.flush_with_scope_check(|scope| -> Result<(), graphmem::StorageError> {
        assert_eq!(scope, "global");
        inserted = Some(
            concurrent
                .remember_with_graph(
                    "concurrent foreign",
                    "observation",
                    0.0,
                    &["repo:/foreign".to_owned()],
                    &[],
                    &[],
                )?
                .id,
        );
        Ok(())
    });
    assert!(
        result.is_err(),
        "flush must not upgrade an obsolete read snapshot"
    );
    assert!(database.get_memory(local.id).unwrap().is_some());
    assert_eq!(
        database
            .get_memory(inserted.unwrap())
            .unwrap()
            .unwrap()
            .content,
        "concurrent foreign"
    );
    drop(concurrent);
    drop(database);
    remove_database(&path);
}

#[test]
fn reopens_and_supports_memory_scope_crud() {
    let (mut database, path) = test_database();
    let memory = database
        .create_memory("  use nextest  ", "convention", 0.8)
        .expect("memory is created");
    assert!(memory.id > 0);
    let repository = database
        .create_scope(" repo:/workspace/project ")
        .expect("scope is created");
    let user = database
        .create_scope("user:test")
        .expect("scope is created");
    assert!(repository.id > 0);
    assert!(user.id > repository.id);

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
    assert!(source.id > 0);
    assert!(target.id > source.id);
    let edge = database
        .create_edge(
            source.id,
            "depends_on",
            target.id,
            Some("runtime dependency"),
        )
        .expect("edge is created");
    assert!(edge.id > 0);

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
fn graph_paths_include_self_relations_without_retraversing_them() {
    let (database, path) = test_database();
    let entity = database.create_entity("component", "API", "api").unwrap();
    let other = database
        .create_entity("component", "Worker", "worker")
        .unwrap();
    let loop_edge = database
        .create_edge(entity.id, "calls_itself", entity.id, None)
        .unwrap();
    database
        .create_edge(entity.id, "calls", other.id, None)
        .unwrap();

    let paths = database
        .graph_paths(entity.id, GraphDirection::Both, 2, 100)
        .unwrap();
    assert_eq!(paths.len(), 3);
    let loop_directions = paths
        .iter()
        .filter(|path| path.hops[0].edge.id == loop_edge.id)
        .map(|path| path.hops[0].direction)
        .collect::<Vec<_>>();
    assert_eq!(loop_directions.len(), 2);
    assert!(loop_directions.contains(&GraphDirection::Outgoing));
    assert!(loop_directions.contains(&GraphDirection::Incoming));
    assert!(paths.iter().all(|path| path.hops.len() == 1));
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

    let result = database.attach_scopes(memory.id, &[scope.id, i64::MAX]);
    assert!(result.is_err());
    assert!(database.list_memory_scopes(memory.id).unwrap().is_empty());

    drop(database);
    remove_database(&path);
}

#[test]
fn remembers_entities_and_relations_atomically() {
    let (mut database, path) = test_database();
    let memory = database
        .remember_with_graph(
            "the api depends on sqlite",
            "fact",
            0.5,
            &["global".to_owned()],
            &[EntityReference {
                kind: "component".to_owned(),
                name: "API".to_owned(),
            }],
            &[Relation {
                source: EntityReference {
                    kind: "component".to_owned(),
                    name: "API".to_owned(),
                },
                relation: "depends_on".to_owned(),
                target: EntityReference {
                    kind: "database".to_owned(),
                    name: "SQLite".to_owned(),
                },
                metadata: None,
            }],
        )
        .expect("memory and graph are created");
    assert_eq!(database.list_memory_scopes(memory.id).unwrap().len(), 1);
    assert_eq!(database.list_memory_entities(memory.id).unwrap().len(), 2);
    assert_eq!(database.list_entities().unwrap().len(), 2);
    assert_eq!(database.list_outgoing_edges(1).unwrap().len(), 1);
    let entity_id = database.list_entities().unwrap()[0].id;
    assert!(database.delete_entity(entity_id).unwrap());
    assert!(database.get_memory(memory.id).unwrap().is_some());
    assert_eq!(database.list_memory_entities(memory.id).unwrap().len(), 1);
    assert!(database.list_all_edges().unwrap().is_empty());
    drop(database);
    remove_database(&path);
}

#[test]
fn default_database_uses_graphmem_home_override() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is valid")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("graphmem-home-{}-{nonce}", std::process::id()));
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

#[test]
fn flush_clears_memory_scopes_and_graph() {
    let (mut database, path) = test_database();
    let memory = database
        .create_memory("temporary memory", "fact", 0.0)
        .expect("memory is created");
    let scope = database.create_scope("global").expect("scope is created");
    database
        .attach_scopes(memory.id, &[scope.id])
        .expect("scope is attached");
    let source = database
        .create_entity("component", "API", "api")
        .expect("source is created");
    let target = database
        .create_entity("database", "SQLite", "sqlite")
        .expect("target is created");
    let edge = database
        .create_edge(source.id, "depends_on", target.id, None)
        .expect("edge is created");

    database.flush().expect("database is flushed");

    assert!(database.list_memories(10).unwrap().is_empty());
    assert!(database.list_scopes().unwrap().is_empty());
    assert!(database.get_entity(source.id).unwrap().is_none());
    assert!(database.get_entity(target.id).unwrap().is_none());
    assert!(database.get_edge(edge.id).unwrap().is_none());
    drop(database);
    remove_database(&path);
}

#[test]
fn migrations_apply_once_and_are_stable_across_reopen() {
    let (database, path) = test_database();
    assert_eq!(database.schema_version().unwrap(), 1);
    drop(database);

    let reopened = Database::open(&path).expect("database reopens");
    assert_eq!(reopened.schema_version().unwrap(), 1);
    drop(reopened);
    remove_database(&path);
}
