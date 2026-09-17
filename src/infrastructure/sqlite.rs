use std::{
    collections::HashMap,
    env, fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use directories::BaseDirs;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use thiserror::Error;

use crate::domain::{
    Edge, Entity, EntityReference, GraphDirection, GraphHop, GraphPath, Memory, Relation, Scope,
    SearchResult, StoreStats, edge_document, entity_document,
};

pub type Result<T> = std::result::Result<T, StorageError>;

const FTS_SYNTAX_CHARACTERS: [char; 10] = ['(', ')', '{', '}', ':', ',', '"', '^', '*', '+'];

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("filesystem error: {0}")]
    Io(#[from] std::io::Error),
    #[error("home directory is unavailable")]
    HomeDirectoryUnavailable,
    #[error("invalid {field}: {message}")]
    Invalid {
        field: &'static str,
        message: &'static str,
    },
    #[error("system clock is before the Unix epoch")]
    Clock,
}

pub struct Database {
    connection: Connection,
    path: PathBuf,
}

/// Embeds the documents of a new memory inside its `remember` transaction.
/// `embed` receives every document at once and must return one vector per
/// document, in order; an error rolls the whole memory back.
pub struct VectorSink<'a, E> {
    pub model: &'a str,
    pub revision: &'a str,
    pub embed: &'a mut EmbedFn<'a, E>,
}

/// Embeds documents in order, one vector per document.
pub type EmbedFn<'a, E> = dyn FnMut(&[&str]) -> std::result::Result<Vec<Vec<f32>>, E> + 'a;

impl Database {
    pub fn default_path() -> Result<PathBuf> {
        Ok(default_data_dir()?.join("memory.sqlite"))
    }

    pub fn open_default() -> Result<Self> {
        let path = Self::default_path()?;
        let data_dir = path.parent().ok_or(StorageError::Invalid {
            field: "database path",
            message: "default database has no parent directory",
        })?;
        create_data_dir(data_dir, true)?;
        Self::open_path(&path)
    }

    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            create_data_dir(parent, false)?;
        }
        Self::open_path(path)
    }

    fn open_path(path: &Path) -> Result<Self> {
        let path = path.to_path_buf();
        let mut connection = Connection::open(&path)?;
        // `remember` embeds inside its transaction, so a concurrent writer can
        // briefly find the database locked; wait instead of failing immediately.
        connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA busy_timeout = 5000;",
        )?;
        ensure_schema(&mut connection)?;
        Ok(Self { connection, path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn stats(&self) -> Result<StoreStats> {
        self.connection
            .query_row(
                "SELECT
                    (SELECT COUNT(*) FROM memories),
                    (SELECT COUNT(*) FROM scopes),
                    (SELECT COUNT(*) FROM entities),
                    (SELECT COUNT(*) FROM edges)",
                [],
                |row| {
                    Ok(StoreStats {
                        memories: row.get(0)?,
                        scopes: row.get(1)?,
                        entities: row.get(2)?,
                        edges: row.get(3)?,
                    })
                },
            )
            .map_err(StorageError::from)
    }

    pub fn create_memory(
        &self,
        content: &str,
        memory_type: &str,
        importance: f64,
    ) -> Result<Memory> {
        validate_text("content", content)?;
        validate_text("memory_type", memory_type)?;
        validate_importance(importance)?;

        let timestamp = now_millis()?;
        self.connection.execute(
            "INSERT INTO memories
             (content, memory_type, importance, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?4)",
            params![content, memory_type.trim(), importance, timestamp],
        )?;
        let id = self.connection.last_insert_rowid();
        self.get_memory(id)?.ok_or(StorageError::Invalid {
            field: "memory",
            message: "memory was not created",
        })
    }

    pub fn remember_with_graph(
        &mut self,
        content: &str,
        memory_type: &str,
        importance: f64,
        scopes: &[String],
        entities: &[EntityReference],
        relations: &[Relation],
    ) -> Result<Memory> {
        self.remember_with_graph_and_vectors::<StorageError>(
            content,
            memory_type,
            importance,
            scopes,
            entities,
            relations,
            None,
        )
    }

    /// Stores a memory with its graph and, when `vectors` is given, the
    /// embeddings of the memory and of any linked entity or edge that has no
    /// vector yet for the sink's model and revision. Everything commits
    /// together, so a failed embedding stores nothing.
    #[allow(clippy::too_many_arguments)]
    pub fn remember_with_graph_and_vectors<E: From<StorageError>>(
        &mut self,
        content: &str,
        memory_type: &str,
        importance: f64,
        scopes: &[String],
        entities: &[EntityReference],
        relations: &[Relation],
        vectors: Option<VectorSink<'_, E>>,
    ) -> std::result::Result<Memory, E> {
        validate_text("content", content)?;
        validate_text("memory_type", memory_type)?;
        validate_importance(importance)?;
        let scopes = scopes
            .iter()
            .map(|scope| normalize_scope(scope))
            .collect::<Result<Vec<_>>>()?;
        let entities = entities
            .iter()
            .map(normalize_entity_reference)
            .collect::<Result<Vec<_>>>()?;
        let relations = relations
            .iter()
            .map(normalize_relation)
            .collect::<Result<Vec<_>>>()?;
        let timestamp = now_millis()?;
        let transaction = self.connection.transaction().map_err(StorageError::from)?;
        let stored = insert_memory_graph(
            &transaction,
            content,
            memory_type,
            importance,
            &scopes,
            &entities,
            &relations,
            timestamp,
        )?;
        if let Some(sink) = vectors {
            store_new_vectors(&transaction, &stored, sink)?;
        }
        transaction.commit().map_err(StorageError::from)?;
        Ok(stored.memory)
    }

    pub fn get_memory(&self, id: i64) -> Result<Option<Memory>> {
        self.connection
            .query_row(
                "SELECT id, content, memory_type, importance, created_at, updated_at,
                        last_accessed_at, access_count
                 FROM memories WHERE id = ?1",
                [id],
                memory_from_row,
            )
            .optional()
            .map_err(StorageError::from)
    }

    pub fn update_memory(
        &self,
        id: i64,
        content: &str,
        memory_type: &str,
        importance: f64,
    ) -> Result<bool> {
        validate_text("content", content)?;
        validate_text("memory_type", memory_type)?;
        validate_importance(importance)?;

        let changed = self.connection.execute(
            "UPDATE memories
             SET content = ?2, memory_type = ?3, importance = ?4, updated_at = ?5
             WHERE id = ?1",
            params![id, content, memory_type.trim(), importance, now_millis()?],
        )?;
        if changed == 1 {
            self.connection
                .execute("DELETE FROM memory_embeddings WHERE memory_id = ?1", [id])?;
        }
        Ok(changed == 1)
    }

    pub fn delete_memory(&self, id: i64) -> Result<bool> {
        Ok(self
            .connection
            .execute("DELETE FROM memories WHERE id = ?1", [id])?
            == 1)
    }

    pub fn flush(&mut self) -> Result<()> {
        self.with_transaction(|transaction| {
            transaction.execute("DELETE FROM memories", [])?;
            transaction.execute("DELETE FROM scopes", [])?;
            transaction.execute("DELETE FROM edges", [])?;
            transaction.execute("DELETE FROM entities", [])?;
            Ok(())
        })
    }

    pub fn list_memories(&self, limit: usize) -> Result<Vec<Memory>> {
        let limit = limit_value(limit)?;
        let mut statement = self.connection.prepare(
            "SELECT id, content, memory_type, importance, created_at, updated_at,
                    last_accessed_at, access_count
             FROM memories ORDER BY created_at DESC, id DESC LIMIT ?1",
        )?;
        let memories = statement
            .query_map([limit], memory_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(memories)
    }

    pub fn list_all_memories(&self) -> Result<Vec<Memory>> {
        let mut statement = self.connection.prepare(
            "SELECT id, content, memory_type, importance, created_at, updated_at,
                    last_accessed_at, access_count
             FROM memories ORDER BY created_at DESC, id DESC",
        )?;
        let memories = statement
            .query_map([], memory_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(memories)
    }

    pub fn list_memories_in_scope(&self, scope: &str, limit: usize) -> Result<Vec<Memory>> {
        let scope = normalize_scope(scope)?;
        let limit = limit_value(limit)?;
        let mut statement = self.connection.prepare(
            "SELECT m.id, m.content, m.memory_type, m.importance, m.created_at, m.updated_at,
                    m.last_accessed_at, m.access_count
             FROM memories m
             JOIN memory_scopes ms ON ms.memory_id = m.id
             JOIN scopes s ON s.id = ms.scope_id
             WHERE s.name = ?1
             ORDER BY m.created_at DESC, m.id DESC LIMIT ?2",
        )?;
        let memories = statement
            .query_map(params![scope, limit], memory_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(memories)
    }

    pub fn list_memories_in_scopes(&self, scopes: &[String]) -> Result<Vec<Memory>> {
        let scopes = normalized_scopes(scopes)?;
        let placeholders = (0..scopes.len())
            .map(|index| format!("?{}", index + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT m.id, m.content, m.memory_type, m.importance, m.created_at, m.updated_at,
                    m.last_accessed_at, m.access_count
             FROM memories m
             WHERE NOT EXISTS (
                 SELECT 1 FROM memory_scopes ms WHERE ms.memory_id = m.id
             ) OR EXISTS (
                 SELECT 1 FROM memory_scopes ms
                 JOIN scopes s ON s.id = ms.scope_id
                 WHERE ms.memory_id = m.id AND (s.name = 'global' OR s.name IN ({placeholders}))
             )
             ORDER BY m.created_at DESC, m.id DESC"
        );
        let mut statement = self.connection.prepare(&sql)?;
        let memories = statement
            .query_map(rusqlite::params_from_iter(scopes), memory_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(memories)
    }

    pub fn search_memories(
        &self,
        query: &str,
        scope: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        validate_text("query", query)?;
        let scope = scope.map(normalize_scope).transpose()?;
        let limit = limit_value(limit)?;
        let mut statement = self.connection.prepare(
            "SELECT m.id, m.content, m.memory_type, m.importance, m.created_at, m.updated_at,
                    m.last_accessed_at, m.access_count, -bm25(memories_fts) AS score
             FROM memories_fts
             JOIN memories m ON m.rowid = memories_fts.rowid
             WHERE memories_fts MATCH ?1
               AND (?2 IS NULL OR EXISTS (
                   SELECT 1 FROM memory_scopes ms
                   JOIN scopes s ON s.id = ms.scope_id
                   WHERE ms.memory_id = m.id AND s.name = ?2
               ))
             ORDER BY bm25(memories_fts) ASC, m.importance DESC, m.updated_at DESC, m.id DESC
             LIMIT ?3",
        )?;
        let memories = run_fts_query(query, |match_query| {
            statement
                .query_map(params![match_query, scope, limit], search_result_from_row)?
                .collect::<rusqlite::Result<Vec<_>>>()
        })?;
        Ok(memories)
    }

    pub fn search_memories_in_scopes(
        &self,
        query: &str,
        scopes: &[String],
        limit: usize,
        proximate_names: &[String],
    ) -> Result<Vec<SearchResult>> {
        validate_text("query", query)?;
        let scopes = normalized_scopes(scopes)?;
        let limit = limit_value(limit)?;
        let placeholders = (0..scopes.len())
            .map(|index| format!("?{}", index + 2))
            .collect::<Vec<_>>()
            .join(", ");
        let rank_start = scopes.len() + 2;
        let rank_placeholders = (0..scopes.len())
            .map(|index| format!("?{}", rank_start + index))
            .collect::<Vec<_>>()
            .join(", ");
        let graph_start = rank_start + scopes.len();
        let graph_clause = if proximate_names.is_empty() {
            String::new()
        } else {
            let likes = (0..proximate_names.len())
                .map(|index| format!("m.content LIKE '%' || ?{} || '%'", graph_start + index))
                .collect::<Vec<_>>()
                .join(" OR ");
            format!("CASE WHEN {likes} THEN 0 ELSE 1 END,")
        };
        let limit_placeholder = format!("?{}", graph_start + proximate_names.len());
        let sql = format!(
            "SELECT m.id, m.content, m.memory_type, m.importance, m.created_at, m.updated_at,
                    m.last_accessed_at, m.access_count, -bm25(memories_fts) AS score
             FROM memories_fts
             JOIN memories m ON m.rowid = memories_fts.rowid
             WHERE memories_fts MATCH ?1
               AND (NOT EXISTS (
                   SELECT 1 FROM memory_scopes ms WHERE ms.memory_id = m.id
               ) OR EXISTS (
                   SELECT 1 FROM memory_scopes ms
                   JOIN scopes s ON s.id = ms.scope_id
                   WHERE ms.memory_id = m.id AND (s.name = 'global' OR s.name IN ({placeholders}))
               ))
             ORDER BY CASE WHEN EXISTS (
                   SELECT 1 FROM memory_scopes ms
                   JOIN scopes s ON s.id = ms.scope_id
                   WHERE ms.memory_id = m.id AND s.name IN ({rank_placeholders})
               ) THEN 0 ELSE 1 END,
               {graph_clause}
               bm25(memories_fts) ASC, m.importance DESC, m.updated_at DESC, m.id DESC
             LIMIT {limit_placeholder}"
        );
        let mut values = Vec::with_capacity(1 + scopes.len() * 2 + proximate_names.len() + 1);
        values.push(rusqlite::types::Value::Text(query.to_owned()));
        values.extend(scopes.iter().cloned().map(rusqlite::types::Value::Text));
        values.extend(scopes.iter().cloned().map(rusqlite::types::Value::Text));
        values.extend(
            proximate_names
                .iter()
                .cloned()
                .map(rusqlite::types::Value::Text),
        );
        values.push(rusqlite::types::Value::Integer(limit));
        let mut statement = self.connection.prepare(&sql)?;
        let memories = run_fts_query(query, |match_query| {
            values[0] = rusqlite::types::Value::Text(match_query.to_owned());
            statement
                .query_map(
                    rusqlite::params_from_iter(values.iter()),
                    search_result_from_row,
                )?
                .collect::<rusqlite::Result<Vec<_>>>()
        })?;
        Ok(memories)
    }

    pub fn create_scope(&self, name: &str) -> Result<Scope> {
        let name = normalize_scope(name)?;
        self.connection.execute(
            "INSERT INTO scopes (name, created_at) VALUES (?1, ?2)",
            params![name, now_millis()?],
        )?;
        let id = self.connection.last_insert_rowid();
        self.get_scope(id)?.ok_or(StorageError::Invalid {
            field: "scope",
            message: "scope was not created",
        })
    }

    pub fn get_scope(&self, id: i64) -> Result<Option<Scope>> {
        self.connection
            .query_row(
                "SELECT id, name, created_at FROM scopes WHERE id = ?1",
                [id],
                scope_from_row,
            )
            .optional()
            .map_err(StorageError::from)
    }

    pub fn get_scope_by_name(&self, name: &str) -> Result<Option<Scope>> {
        let name = normalize_scope(name)?;
        self.connection
            .query_row(
                "SELECT id, name, created_at FROM scopes WHERE name = ?1",
                [name],
                scope_from_row,
            )
            .optional()
            .map_err(StorageError::from)
    }

    pub fn delete_scope(&self, id: i64) -> Result<bool> {
        Ok(self
            .connection
            .execute("DELETE FROM scopes WHERE id = ?1", [id])?
            == 1)
    }

    pub fn list_scopes(&self) -> Result<Vec<Scope>> {
        let mut statement = self
            .connection
            .prepare("SELECT id, name, created_at FROM scopes ORDER BY name")?;
        let scopes = statement
            .query_map([], scope_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(scopes)
    }

    pub fn attach_scopes(&mut self, memory_id: i64, scope_ids: &[i64]) -> Result<()> {
        self.with_transaction(|transaction| {
            for scope_id in scope_ids {
                transaction.execute(
                    "INSERT INTO memory_scopes (memory_id, scope_id) VALUES (?1, ?2)",
                    params![memory_id, scope_id],
                )?;
            }
            Ok(())
        })
    }

    pub fn detach_scope(&self, memory_id: i64, scope_id: i64) -> Result<bool> {
        Ok(self.connection.execute(
            "DELETE FROM memory_scopes WHERE memory_id = ?1 AND scope_id = ?2",
            params![memory_id, scope_id],
        )? == 1)
    }

    pub fn list_memory_scopes(&self, memory_id: i64) -> Result<Vec<Scope>> {
        let mut statement = self.connection.prepare(
            "SELECT s.id, s.name, s.created_at
             FROM scopes s
             JOIN memory_scopes ms ON ms.scope_id = s.id
             WHERE ms.memory_id = ?1 ORDER BY s.name",
        )?;
        let scopes = statement
            .query_map([memory_id], scope_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(scopes)
    }

    pub fn list_memory_entities(&self, memory_id: i64) -> Result<Vec<Entity>> {
        let mut statement = self.connection.prepare(
            "SELECT e.id, e.kind, e.name, e.canonical_name, e.created_at, e.updated_at
             FROM entities e
             JOIN memory_entities me ON me.entity_id = e.id
             WHERE me.memory_id = ?1 ORDER BY e.id",
        )?;
        let entities = statement
            .query_map([memory_id], entity_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(entities)
    }

    pub fn entities_by_memory(&self, memory_ids: &[i64]) -> Result<HashMap<i64, Vec<Entity>>> {
        let mut grouped = HashMap::new();
        if memory_ids.is_empty() {
            return Ok(grouped);
        }
        let placeholders = (0..memory_ids.len())
            .map(|index| format!("?{}", index + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT e.id, e.kind, e.name, e.canonical_name, e.created_at, e.updated_at, me.memory_id
             FROM entities e
             JOIN memory_entities me ON me.entity_id = e.id
             WHERE me.memory_id IN ({placeholders}) ORDER BY me.memory_id, e.id"
        );
        let mut statement = self.connection.prepare(&sql)?;
        let rows = statement
            .query_map(rusqlite::params_from_iter(memory_ids), |row| {
                Ok((row.get::<_, i64>(6)?, entity_from_row(row)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (memory_id, entity) in rows {
            grouped
                .entry(memory_id)
                .or_insert_with(Vec::new)
                .push(entity);
        }
        Ok(grouped)
    }

    pub fn memory_embedding(
        &self,
        memory_id: i64,
        model: &str,
        revision: &str,
    ) -> Result<Option<Vec<f32>>> {
        read_embedding(
            &self.connection,
            "memory_embeddings",
            "memory_id",
            memory_id,
            model,
            revision,
        )
    }

    pub fn store_memory_embedding(
        &self,
        memory_id: i64,
        model: &str,
        revision: &str,
        vector: &[f32],
    ) -> Result<()> {
        write_embedding(
            &self.connection,
            "memory_embeddings",
            "memory_id",
            memory_id,
            model,
            revision,
            vector,
        )
    }

    pub fn create_entity(&self, kind: &str, name: &str, canonical_name: &str) -> Result<Entity> {
        validate_text("kind", kind)?;
        validate_text("name", name)?;
        validate_text("canonical_name", canonical_name)?;

        let timestamp = now_millis()?;
        self.connection.execute(
            "INSERT INTO entities
             (kind, name, canonical_name, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?4)",
            params![kind.trim(), name.trim(), canonical_name.trim(), timestamp],
        )?;
        let id = self.connection.last_insert_rowid();
        self.get_entity(id)?.ok_or(StorageError::Invalid {
            field: "entity",
            message: "entity was not created",
        })
    }

    pub fn list_entities(&self) -> Result<Vec<Entity>> {
        let mut statement = self.connection.prepare(
            "SELECT id, kind, name, canonical_name, created_at, updated_at
             FROM entities ORDER BY id",
        )?;
        let entities = statement
            .query_map([], entity_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(entities)
    }

    pub fn get_entity(&self, id: i64) -> Result<Option<Entity>> {
        self.connection
            .query_row(
                "SELECT id, kind, name, canonical_name, created_at, updated_at
                 FROM entities WHERE id = ?1",
                [id],
                entity_from_row,
            )
            .optional()
            .map_err(StorageError::from)
    }

    pub fn get_entity_by_canonical(
        &self,
        kind: &str,
        canonical_name: &str,
    ) -> Result<Option<Entity>> {
        validate_text("kind", kind)?;
        validate_text("canonical_name", canonical_name)?;
        self.connection
            .query_row(
                "SELECT id, kind, name, canonical_name, created_at, updated_at
                 FROM entities WHERE kind = ?1 AND canonical_name = ?2",
                params![kind.trim(), canonical_name.trim()],
                entity_from_row,
            )
            .optional()
            .map_err(StorageError::from)
    }

    pub fn update_entity(
        &self,
        id: i64,
        kind: &str,
        name: &str,
        canonical_name: &str,
    ) -> Result<bool> {
        validate_text("kind", kind)?;
        validate_text("name", name)?;
        validate_text("canonical_name", canonical_name)?;
        Ok(self.connection.execute(
            "UPDATE entities
             SET kind = ?2, name = ?3, canonical_name = ?4, updated_at = ?5
             WHERE id = ?1",
            params![
                id,
                kind.trim(),
                name.trim(),
                canonical_name.trim(),
                now_millis()?
            ],
        )? == 1)
    }

    pub fn delete_entity(&self, id: i64) -> Result<bool> {
        Ok(self
            .connection
            .execute("DELETE FROM entities WHERE id = ?1", [id])?
            == 1)
    }

    pub fn create_edge(
        &self,
        source_id: i64,
        relation: &str,
        target_id: i64,
        metadata: Option<&str>,
    ) -> Result<Edge> {
        validate_text("relation", relation)?;
        self.connection.execute(
            "INSERT INTO edges (source_id, relation, target_id, created_at, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                source_id,
                relation.trim(),
                target_id,
                now_millis()?,
                metadata
            ],
        )?;
        let id = self.connection.last_insert_rowid();
        self.get_edge(id)?.ok_or(StorageError::Invalid {
            field: "edge",
            message: "edge was not created",
        })
    }

    pub fn get_edge(&self, id: i64) -> Result<Option<Edge>> {
        self.connection
            .query_row(
                "SELECT id, source_id, relation, target_id, created_at, metadata
                 FROM edges WHERE id = ?1",
                [id],
                edge_from_row,
            )
            .optional()
            .map_err(StorageError::from)
    }

    pub fn list_all_edges(&self) -> Result<Vec<Edge>> {
        let mut statement = self.connection.prepare(
            "SELECT id, source_id, relation, target_id, created_at, metadata FROM edges ORDER BY id",
        )?;
        let edges = statement
            .query_map([], edge_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(edges)
    }

    pub fn edge_embedding(
        &self,
        edge_id: i64,
        model: &str,
        revision: &str,
    ) -> Result<Option<Vec<f32>>> {
        read_embedding(
            &self.connection,
            "edge_embeddings",
            "edge_id",
            edge_id,
            model,
            revision,
        )
    }

    pub fn store_edge_embedding(
        &self,
        edge_id: i64,
        model: &str,
        revision: &str,
        vector: &[f32],
    ) -> Result<()> {
        write_embedding(
            &self.connection,
            "edge_embeddings",
            "edge_id",
            edge_id,
            model,
            revision,
            vector,
        )
    }

    pub fn entity_embedding(
        &self,
        entity_id: i64,
        model: &str,
        revision: &str,
    ) -> Result<Option<Vec<f32>>> {
        read_embedding(
            &self.connection,
            "entity_embeddings",
            "entity_id",
            entity_id,
            model,
            revision,
        )
    }

    pub fn store_entity_embedding(
        &self,
        entity_id: i64,
        model: &str,
        revision: &str,
        vector: &[f32],
    ) -> Result<()> {
        write_embedding(
            &self.connection,
            "entity_embeddings",
            "entity_id",
            entity_id,
            model,
            revision,
            vector,
        )
    }

    pub fn get_edge_by_endpoints(
        &self,
        source_id: i64,
        relation: &str,
        target_id: i64,
    ) -> Result<Option<Edge>> {
        validate_text("relation", relation)?;
        self.connection
            .query_row(
                "SELECT id, source_id, relation, target_id, created_at, metadata
                 FROM edges
                 WHERE source_id = ?1 AND relation = ?2 AND target_id = ?3
                 ORDER BY id LIMIT 1",
                params![source_id, relation.trim(), target_id],
                edge_from_row,
            )
            .optional()
            .map_err(StorageError::from)
    }

    pub fn update_edge(&self, id: i64, relation: &str, metadata: Option<&str>) -> Result<bool> {
        validate_text("relation", relation)?;
        Ok(self.connection.execute(
            "UPDATE edges SET relation = ?2, metadata = ?3 WHERE id = ?1",
            params![id, relation.trim(), metadata],
        )? == 1)
    }

    pub fn delete_edge(&self, id: i64) -> Result<bool> {
        Ok(self
            .connection
            .execute("DELETE FROM edges WHERE id = ?1", [id])?
            == 1)
    }

    pub fn list_outgoing_edges(&self, source_id: i64) -> Result<Vec<Edge>> {
        self.list_edges(
            "SELECT id, source_id, relation, target_id, created_at, metadata
             FROM edges WHERE source_id = ?1 ORDER BY created_at, id",
            source_id,
        )
    }

    pub fn list_incoming_edges(&self, target_id: i64) -> Result<Vec<Edge>> {
        self.list_edges(
            "SELECT id, source_id, relation, target_id, created_at, metadata
             FROM edges WHERE target_id = ?1 ORDER BY created_at, id",
            target_id,
        )
    }

    pub fn graph_paths(
        &self,
        entity_id: i64,
        direction: GraphDirection,
        max_depth: usize,
        limit: usize,
    ) -> Result<Vec<GraphPath>> {
        let max_depth = i64::try_from(max_depth).map_err(|_| StorageError::Invalid {
            field: "max_depth",
            message: "is too large",
        })?;
        let limit = limit_value(limit)?;
        let steps = match direction {
            GraphDirection::Outgoing => {
                "SELECT source_id AS from_id, target_id AS node_id, id AS edge_id, 'outgoing' AS direction FROM edges"
            }
            GraphDirection::Incoming => {
                "SELECT target_id AS from_id, source_id AS node_id, id AS edge_id, 'incoming' AS direction FROM edges"
            }
            GraphDirection::Both => {
                "SELECT source_id AS from_id, target_id AS node_id, id AS edge_id, 'outgoing' AS direction FROM edges
                 UNION ALL
                 SELECT target_id AS from_id, source_id AS node_id, id AS edge_id, 'incoming' AS direction FROM edges"
            }
        };
        let sql = format!(
            "WITH RECURSIVE paths(node_id, depth, nodes, edge_ids, directions) AS (
                 VALUES (?1, 0, printf(',%d,', ?1), '', '')
                 UNION ALL
                 SELECT step.node_id,
                        paths.depth + 1,
                        paths.nodes || step.node_id || ',',
                        paths.edge_ids || step.edge_id || ',',
                        paths.directions || step.direction || ','
                 FROM paths
                 JOIN ({steps}) AS step ON step.from_id = paths.node_id
                 WHERE paths.depth < ?2
                   AND instr(paths.nodes, printf(',%d,', step.node_id)) = 0
             )
             SELECT edge_ids, directions
             FROM paths
             WHERE depth > 0
             ORDER BY depth, edge_ids
             LIMIT ?3"
        );
        let mut statement = self.connection.prepare(&sql)?;
        let rows = statement
            .query_map(params![entity_id, max_depth, limit], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|(edge_ids, directions)| self.graph_path(&edge_ids, &directions))
            .collect()
    }

    fn list_edges(&self, sql: &str, id: i64) -> Result<Vec<Edge>> {
        let mut statement = self.connection.prepare(sql)?;
        let edges = statement
            .query_map([id], edge_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(edges)
    }

    fn graph_path(&self, edge_ids: &str, directions: &str) -> Result<GraphPath> {
        let edge_ids = edge_ids
            .split(',')
            .filter(|id| !id.is_empty())
            .map(|id| {
                id.parse::<i64>().map_err(|_| StorageError::Invalid {
                    field: "graph path",
                    message: "contains an invalid edge id",
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let directions = directions
            .split(',')
            .filter(|direction| !direction.is_empty())
            .map(graph_direction)
            .collect::<Result<Vec<_>>>()?;
        if edge_ids.len() != directions.len() {
            return Err(StorageError::Invalid {
                field: "graph path",
                message: "has mismatched edges and directions",
            });
        }
        let hops = edge_ids
            .into_iter()
            .zip(directions)
            .map(|(edge_id, direction)| {
                let edge = self.get_edge(edge_id)?.ok_or(StorageError::Invalid {
                    field: "graph path",
                    message: "references a missing edge",
                })?;
                let entity_id = match direction {
                    GraphDirection::Outgoing => edge.target_id,
                    GraphDirection::Incoming => edge.source_id,
                    GraphDirection::Both => unreachable!(),
                };
                let entity = self.get_entity(entity_id)?.ok_or(StorageError::Invalid {
                    field: "graph path",
                    message: "references a missing entity",
                })?;
                Ok(GraphHop {
                    entity,
                    edge,
                    direction,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(GraphPath { hops })
    }

    fn with_transaction<F, T>(&mut self, operation: F) -> Result<T>
    where
        F: FnOnce(&Transaction<'_>) -> Result<T>,
    {
        let transaction = self.connection.transaction()?;
        let result = operation(&transaction)?;
        transaction.commit()?;
        Ok(result)
    }
}

fn default_data_dir() -> Result<PathBuf> {
    if let Some(path) = env::var_os("GRAPHMEM_HOME").filter(|path| !path.is_empty()) {
        return Ok(PathBuf::from(path));
    }

    BaseDirs::new()
        .map(|directories| directories.home_dir().join(".graphmem"))
        .ok_or(StorageError::HomeDirectoryUnavailable)
}

fn create_data_dir(path: &Path, restrict_permissions: bool) -> Result<()> {
    fs::create_dir_all(path)?;
    if restrict_permissions {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mut permissions = fs::metadata(path)?.permissions();
            permissions.set_mode(0o700);
            fs::set_permissions(path, permissions)?;
        }
    }
    Ok(())
}

fn ensure_schema(connection: &mut Connection) -> Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS memories (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 content TEXT NOT NULL,
                 memory_type TEXT NOT NULL,
                 importance REAL NOT NULL DEFAULT 0.0,
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL,
                 last_accessed_at INTEGER,
                 access_count INTEGER NOT NULL DEFAULT 0 CHECK (access_count >= 0)
             );
             CREATE TABLE IF NOT EXISTS scopes (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 name TEXT NOT NULL UNIQUE,
                 created_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS memory_scopes (
                 memory_id INTEGER NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
                 scope_id INTEGER NOT NULL REFERENCES scopes(id) ON DELETE CASCADE,
                 PRIMARY KEY (memory_id, scope_id)
             );
             CREATE INDEX IF NOT EXISTS idx_memory_scopes_scope_id ON memory_scopes(scope_id);
             CREATE TABLE IF NOT EXISTS entities (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 kind TEXT NOT NULL,
                 name TEXT NOT NULL,
                 canonical_name TEXT NOT NULL,
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL,
                 UNIQUE (kind, canonical_name)
             );
             CREATE TABLE IF NOT EXISTS edges (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 source_id INTEGER NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
                 relation TEXT NOT NULL,
                 target_id INTEGER NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
                 created_at INTEGER NOT NULL,
                 metadata TEXT
             );
             CREATE INDEX IF NOT EXISTS idx_edges_source_id ON edges(source_id);
             CREATE INDEX IF NOT EXISTS idx_edges_target_id ON edges(target_id);
             CREATE INDEX IF NOT EXISTS idx_edges_relation ON edges(relation);
             CREATE TABLE IF NOT EXISTS memory_entities (
                 memory_id INTEGER NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
                 entity_id INTEGER NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
                 PRIMARY KEY (memory_id, entity_id)
             );
             CREATE INDEX IF NOT EXISTS idx_memory_entities_entity_id ON memory_entities(entity_id);
             CREATE TABLE IF NOT EXISTS memory_embeddings (
                 memory_id INTEGER PRIMARY KEY REFERENCES memories(id) ON DELETE CASCADE,
                 model TEXT NOT NULL,
                 revision TEXT NOT NULL,
                 dimensions INTEGER NOT NULL,
                 vector BLOB NOT NULL
             );
             CREATE TABLE IF NOT EXISTS edge_embeddings (
                 edge_id INTEGER PRIMARY KEY REFERENCES edges(id) ON DELETE CASCADE,
                 model TEXT NOT NULL,
                 revision TEXT NOT NULL,
                 dimensions INTEGER NOT NULL,
                 vector BLOB NOT NULL
             );
             CREATE TABLE IF NOT EXISTS entity_embeddings (
                 entity_id INTEGER PRIMARY KEY REFERENCES entities(id) ON DELETE CASCADE,
                 model TEXT NOT NULL,
                 revision TEXT NOT NULL,
                 dimensions INTEGER NOT NULL,
                 vector BLOB NOT NULL
             );
             CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
                 content,
                 content='memories',
                 content_rowid='rowid'
             );
             INSERT INTO memories_fts(memories_fts) VALUES ('rebuild');
             CREATE TRIGGER IF NOT EXISTS memories_ai AFTER INSERT ON memories BEGIN
                 INSERT INTO memories_fts(rowid, content) VALUES (new.rowid, new.content);
             END;
             CREATE TRIGGER IF NOT EXISTS memories_ad AFTER DELETE ON memories BEGIN
                 INSERT INTO memories_fts(memories_fts, rowid, content)
                 VALUES ('delete', old.rowid, old.content);
             END;
             CREATE TRIGGER IF NOT EXISTS memories_au AFTER UPDATE OF content ON memories BEGIN
                 INSERT INTO memories_fts(memories_fts, rowid, content)
                 VALUES ('delete', old.rowid, old.content);
                 INSERT INTO memories_fts(rowid, content) VALUES (new.rowid, new.content);
             END;",
    )?;
    Ok(())
}

fn search_result_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SearchResult> {
    Ok(SearchResult {
        memory: memory_from_row(row)?,
        score: row.get(8)?,
    })
}

fn memory_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Memory> {
    Ok(Memory {
        id: row.get(0)?,
        content: row.get(1)?,
        memory_type: row.get(2)?,
        importance: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
        last_accessed_at: row.get(6)?,
        access_count: row.get(7)?,
    })
}

fn scope_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Scope> {
    Ok(Scope {
        id: row.get(0)?,
        name: row.get(1)?,
        created_at: row.get(2)?,
    })
}

fn entity_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Entity> {
    Ok(Entity {
        id: row.get(0)?,
        kind: row.get(1)?,
        name: row.get(2)?,
        canonical_name: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

#[derive(Debug)]
struct NormalizedEntityReference {
    kind: String,
    name: String,
    canonical_name: String,
}

#[derive(Debug)]
struct NormalizedRelation {
    source: NormalizedEntityReference,
    relation: String,
    target: NormalizedEntityReference,
    metadata: Option<String>,
}

fn normalize_entity_reference(entity: &EntityReference) -> Result<NormalizedEntityReference> {
    validate_text("kind", &entity.kind)?;
    validate_text("name", &entity.name)?;
    let name = entity.name.trim().to_owned();
    Ok(NormalizedEntityReference {
        kind: entity.kind.trim().to_lowercase(),
        canonical_name: name.to_lowercase(),
        name,
    })
}

fn normalize_relation(relation: &Relation) -> Result<NormalizedRelation> {
    validate_text("relation", &relation.relation)?;
    Ok(NormalizedRelation {
        source: normalize_entity_reference(&relation.source)?,
        relation: relation.relation.trim().to_owned(),
        target: normalize_entity_reference(&relation.target)?,
        metadata: relation.metadata.clone(),
    })
}

fn get_or_create_entity(
    transaction: &Transaction<'_>,
    entity: &NormalizedEntityReference,
    timestamp: i64,
) -> Result<Entity> {
    if let Some(entity) = transaction
        .query_row(
            "SELECT id, kind, name, canonical_name, created_at, updated_at
             FROM entities WHERE kind = ?1 AND canonical_name = ?2",
            params![entity.kind, entity.canonical_name],
            entity_from_row,
        )
        .optional()?
    {
        return Ok(entity);
    }
    transaction.execute(
        "INSERT INTO entities (kind, name, canonical_name, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?4)",
        params![entity.kind, entity.name, entity.canonical_name, timestamp],
    )?;
    transaction
        .query_row(
            "SELECT id, kind, name, canonical_name, created_at, updated_at
             FROM entities WHERE id = ?1",
            [transaction.last_insert_rowid()],
            entity_from_row,
        )
        .map_err(StorageError::from)
}

fn get_or_create_edge(
    transaction: &Transaction<'_>,
    source_id: i64,
    relation: &NormalizedRelation,
    target_id: i64,
    timestamp: i64,
) -> Result<Edge> {
    let select = |transaction: &Transaction<'_>| {
        transaction
            .query_row(
                "SELECT id, source_id, relation, target_id, created_at, metadata
                 FROM edges WHERE source_id = ?1 AND relation = ?2 AND target_id = ?3",
                params![source_id, relation.relation, target_id],
                edge_from_row,
            )
            .optional()
    };
    if let Some(edge) = select(transaction)? {
        return Ok(edge);
    }
    transaction.execute(
        "INSERT INTO edges (source_id, relation, target_id, created_at, metadata)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            source_id,
            relation.relation,
            target_id,
            timestamp,
            relation.metadata
        ],
    )?;
    select(transaction)?.ok_or(StorageError::Sqlite(rusqlite::Error::QueryReturnedNoRows))
}

struct StoredMemoryGraph {
    memory: Memory,
    entities: Vec<Entity>,
    edges: Vec<(Edge, Entity, Entity)>,
}

#[allow(clippy::too_many_arguments)]
fn insert_memory_graph(
    transaction: &Transaction<'_>,
    content: &str,
    memory_type: &str,
    importance: f64,
    scopes: &[String],
    entities: &[NormalizedEntityReference],
    relations: &[NormalizedRelation],
    timestamp: i64,
) -> Result<StoredMemoryGraph> {
    transaction.execute(
        "INSERT INTO memories
         (content, memory_type, importance, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?4)",
        params![content, memory_type.trim(), importance, timestamp],
    )?;
    let memory_id = transaction.last_insert_rowid();
    for scope in scopes {
        transaction.execute(
            "INSERT OR IGNORE INTO scopes (name, created_at) VALUES (?1, ?2)",
            params![scope, timestamp],
        )?;
        let scope_id =
            transaction.query_row("SELECT id FROM scopes WHERE name = ?1", [scope], |row| {
                row.get::<_, i64>(0)
            })?;
        transaction.execute(
            "INSERT OR IGNORE INTO memory_scopes (memory_id, scope_id) VALUES (?1, ?2)",
            params![memory_id, scope_id],
        )?;
    }
    let mut linked = Vec::new();
    for entity in entities {
        let entity = get_or_create_entity(transaction, entity, timestamp)?;
        link_memory_entity(transaction, memory_id, entity.id)?;
        linked.push(entity);
    }
    let mut edges = Vec::new();
    for relation in relations {
        let source = get_or_create_entity(transaction, &relation.source, timestamp)?;
        let target = get_or_create_entity(transaction, &relation.target, timestamp)?;
        let edge = get_or_create_edge(transaction, source.id, relation, target.id, timestamp)?;
        link_memory_entity(transaction, memory_id, source.id)?;
        link_memory_entity(transaction, memory_id, target.id)?;
        linked.push(source.clone());
        linked.push(target.clone());
        edges.push((edge, source, target));
    }
    linked.sort_by_key(|entity| entity.id);
    linked.dedup_by_key(|entity| entity.id);
    let memory = transaction.query_row(
        "SELECT id, content, memory_type, importance, created_at, updated_at,
                last_accessed_at, access_count
         FROM memories WHERE id = ?1",
        [memory_id],
        memory_from_row,
    )?;
    Ok(StoredMemoryGraph {
        memory,
        entities: linked,
        edges,
    })
}

fn store_new_vectors<E: From<StorageError>>(
    connection: &Connection,
    stored: &StoredMemoryGraph,
    sink: VectorSink<'_, E>,
) -> std::result::Result<(), E> {
    let (model, revision) = (sink.model, sink.revision);
    let mut targets = vec![("memory_embeddings", "memory_id", stored.memory.id)];
    let mut documents = vec![stored.memory.content.clone()];
    for entity in &stored.entities {
        let table = ("entity_embeddings", "entity_id", entity.id);
        if read_embedding(connection, table.0, table.1, table.2, model, revision)?.is_none() {
            targets.push(table);
            documents.push(entity_document(entity));
        }
    }
    for (edge, source, target) in &stored.edges {
        let table = ("edge_embeddings", "edge_id", edge.id);
        if read_embedding(connection, table.0, table.1, table.2, model, revision)?.is_none() {
            targets.push(table);
            documents.push(edge_document(edge, source, target));
        }
    }
    let documents = documents.iter().map(String::as_str).collect::<Vec<_>>();
    let vectors = (sink.embed)(&documents)?;
    if vectors.len() != targets.len() {
        return Err(StorageError::Invalid {
            field: "embedding",
            message: "model returned a different number of vectors than documents",
        }
        .into());
    }
    for ((table, column, id), vector) in targets.into_iter().zip(&vectors) {
        write_embedding(connection, table, column, id, model, revision, vector)?;
    }
    Ok(())
}

fn read_embedding(
    connection: &Connection,
    table: &str,
    id_column: &str,
    id: i64,
    model: &str,
    revision: &str,
) -> Result<Option<Vec<f32>>> {
    let sql = format!(
        "SELECT dimensions, vector FROM {table} WHERE {id_column} = ?1 AND model = ?2 AND revision = ?3"
    );
    let embedding = connection
        .query_row(&sql, params![id, model, revision], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
        })
        .optional()
        .map_err(StorageError::from)?;
    embedding
        .map(|(dimensions, bytes)| decode_vector(&bytes, dimensions))
        .transpose()
}

fn write_embedding(
    connection: &Connection,
    table: &str,
    id_column: &str,
    id: i64,
    model: &str,
    revision: &str,
    vector: &[f32],
) -> Result<()> {
    if vector.is_empty() {
        return Err(StorageError::Invalid {
            field: "embedding",
            message: "must not be empty",
        });
    }
    let sql = format!(
        "INSERT INTO {table} ({id_column}, model, revision, dimensions, vector)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT({id_column}) DO UPDATE SET
             model = excluded.model,
             revision = excluded.revision,
             dimensions = excluded.dimensions,
             vector = excluded.vector"
    );
    connection.execute(
        &sql,
        params![
            id,
            model,
            revision,
            vector.len() as i64,
            encode_vector(vector)
        ],
    )?;
    Ok(())
}

fn link_memory_entity(transaction: &Transaction<'_>, memory_id: i64, entity_id: i64) -> Result<()> {
    transaction.execute(
        "INSERT OR IGNORE INTO memory_entities (memory_id, entity_id) VALUES (?1, ?2)",
        params![memory_id, entity_id],
    )?;
    Ok(())
}

fn edge_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Edge> {
    Ok(Edge {
        id: row.get(0)?,
        source_id: row.get(1)?,
        relation: row.get(2)?,
        target_id: row.get(3)?,
        created_at: row.get(4)?,
        metadata: row.get(5)?,
    })
}

fn graph_direction(value: &str) -> Result<GraphDirection> {
    match value {
        "incoming" => Ok(GraphDirection::Incoming),
        "outgoing" => Ok(GraphDirection::Outgoing),
        _ => Err(StorageError::Invalid {
            field: "graph path",
            message: "contains an invalid direction",
        }),
    }
}

fn validate_text(field: &'static str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(StorageError::Invalid {
            field,
            message: "must not be empty",
        });
    }
    Ok(())
}

fn validate_importance(value: f64) -> Result<()> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(StorageError::Invalid {
            field: "importance",
            message: "must be finite",
        })
    }
}

fn limit_value(value: usize) -> Result<i64> {
    if value == 0 {
        return Err(StorageError::Invalid {
            field: "limit",
            message: "must be greater than zero",
        });
    }
    i64::try_from(value).map_err(|_| StorageError::Invalid {
        field: "limit",
        message: "is too large",
    })
}

fn normalize_scope(value: &str) -> Result<String> {
    validate_text("scope", value)?;
    Ok(value.trim().to_owned())
}

fn normalized_scopes(scopes: &[String]) -> Result<Vec<String>> {
    let mut scopes = scopes
        .iter()
        .map(|scope| normalize_scope(scope))
        .collect::<Result<Vec<_>>>()?;
    scopes.sort();
    scopes.dedup();
    if scopes.is_empty() {
        scopes.push("global".to_owned());
    }
    Ok(scopes)
}

fn encode_vector(vector: &[f32]) -> Vec<u8> {
    vector
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn decode_vector(bytes: &[u8], dimensions: i64) -> Result<Vec<f32>> {
    let dimensions = usize::try_from(dimensions).map_err(|_| StorageError::Invalid {
        field: "embedding",
        message: "has invalid dimensions",
    })?;
    let (chunks, remainder) = bytes.as_chunks::<4>();
    if dimensions == 0 || chunks.len() != dimensions || !remainder.is_empty() {
        return Err(StorageError::Invalid {
            field: "embedding",
            message: "has invalid vector data",
        });
    }
    Ok(chunks
        .iter()
        .map(|bytes| f32::from_le_bytes(*bytes))
        .collect())
}

fn run_fts_query<T>(
    query: &str,
    mut execute: impl FnMut(&str) -> rusqlite::Result<T>,
) -> rusqlite::Result<T> {
    match execute(&fts_match_query(query)) {
        Err(rusqlite::Error::SqliteFailure(error, _))
            if error.code == rusqlite::ErrorCode::Unknown =>
        {
            let escaped = format!("\"{}\"", query.replace('"', "\"\""));
            execute(&escaped)
        }
        result => result,
    }
}

/// Plain-language queries match any of their words, so bm25 can rank memories
/// that share most of a question's terms; FTS5 would otherwise require all of
/// them. Quoted phrases and `prefix*` terms stay as written inside that OR, and
/// other punctuation separates words the way the FTS tokenizer does
/// ("Derrickson's" still matches "Derrickson"). A query with an uppercase
/// AND, OR, NOT, or NEAR operator keeps its exact FTS5 semantics.
fn fts_match_query(query: &str) -> String {
    if uses_fts_operators(query) {
        return quote_bareword_terms(query);
    }
    let mut terms = Vec::new();
    for (index, part) in query.split('"').enumerate() {
        if index % 2 == 1 {
            if part.chars().any(char::is_alphanumeric) {
                terms.push(format!("\"{part}\""));
            }
            continue;
        }
        for chunk in part.split(|character: char| !is_bareword(character) && character != '*') {
            let prefix = chunk.ends_with('*');
            let words = chunk
                .split('*')
                .filter(|word| !word.is_empty())
                .collect::<Vec<_>>();
            for (position, word) in words.iter().enumerate() {
                let word = if matches!(*word, "AND" | "OR" | "NOT" | "NEAR") {
                    format!("\"{word}\"")
                } else {
                    (*word).to_owned()
                };
                if prefix && position + 1 == words.len() {
                    terms.push(format!("{word}*"));
                } else {
                    terms.push(word);
                }
            }
        }
    }
    if terms.is_empty() {
        return quote_bareword_terms(query);
    }
    terms.join(" OR ")
}

fn uses_fts_operators(query: &str) -> bool {
    query
        .split_whitespace()
        .any(|term| matches!(term, "AND" | "OR" | "NOT" | "NEAR") || term.starts_with("NEAR("))
}

fn quote_bareword_terms(query: &str) -> String {
    query
        .split_whitespace()
        .map(|term| {
            if needs_quoting(term) {
                format!("\"{term}\"")
            } else {
                term.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn needs_quoting(term: &str) -> bool {
    !term
        .chars()
        .any(|character| FTS_SYNTAX_CHARACTERS.contains(&character))
        && term.chars().any(|character| !is_bareword(character))
}

fn is_bareword(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_' || !character.is_ascii()
}

fn now_millis() -> Result<i64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| StorageError::Clock)
        .and_then(|duration| i64::try_from(duration.as_millis()).map_err(|_| StorageError::Clock))
}
