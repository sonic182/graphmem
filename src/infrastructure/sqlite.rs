use std::{
    env, fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use directories::BaseDirs;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use thiserror::Error;
use uuid::Uuid;

use crate::domain::{Edge, Entity, Memory, Scope};

const SCHEMA_VERSION: i64 = 1;

pub type Result<T> = std::result::Result<T, StorageError>;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("filesystem error: {0}")]
    Io(#[from] std::io::Error),
    #[error("home directory is unavailable")]
    HomeDirectoryUnavailable,
    #[error("database schema version {0} is newer than supported version {SCHEMA_VERSION}")]
    UnsupportedSchema(i64),
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
        connection.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
        migrate(&mut connection)?;
        Ok(Self { connection, path })
    }

    pub fn path(&self) -> &Path {
        &self.path
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

        let id = Uuid::now_v7();
        let timestamp = now_millis()?;
        self.connection.execute(
            "INSERT INTO memories
             (id, content, memory_type, importance, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![
                id.to_string(),
                content,
                memory_type.trim(),
                importance,
                timestamp
            ],
        )?;
        self.get_memory(id)?.ok_or(StorageError::Invalid {
            field: "memory",
            message: "memory was not created",
        })
    }

    pub fn get_memory(&self, id: Uuid) -> Result<Option<Memory>> {
        self.connection
            .query_row(
                "SELECT id, content, memory_type, importance, created_at, updated_at,
                        last_accessed_at, access_count
                 FROM memories WHERE id = ?1",
                [id.to_string()],
                memory_from_row,
            )
            .optional()
            .map_err(StorageError::from)
    }

    pub fn update_memory(
        &self,
        id: Uuid,
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
            params![
                id.to_string(),
                content,
                memory_type.trim(),
                importance,
                now_millis()?
            ],
        )?;
        Ok(changed == 1)
    }

    pub fn delete_memory(&self, id: Uuid) -> Result<bool> {
        Ok(self
            .connection
            .execute("DELETE FROM memories WHERE id = ?1", [id.to_string()])?
            == 1)
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

    // ponytail: LIKE scan; replace with FTS5 ranking when Phase 3 needs scale.
    pub fn search_memories(
        &self,
        query: &str,
        scope: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Memory>> {
        let pattern = like_pattern(query)?;
        let limit = limit_value(limit)?;
        if let Some(scope) = scope {
            let scope = normalize_scope(scope)?;
            let mut statement = self.connection.prepare(
                "SELECT m.id, m.content, m.memory_type, m.importance, m.created_at, m.updated_at,
                        m.last_accessed_at, m.access_count
                 FROM memories m
                 JOIN memory_scopes ms ON ms.memory_id = m.id
                 JOIN scopes s ON s.id = ms.scope_id
                 WHERE m.content LIKE ?1 ESCAPE '\\' AND s.name = ?2
                 ORDER BY m.created_at DESC, m.id DESC LIMIT ?3",
            )?;
            let memories = statement
                .query_map(params![pattern, scope, limit], memory_from_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            return Ok(memories);
        }

        let mut statement = self.connection.prepare(
            "SELECT id, content, memory_type, importance, created_at, updated_at,
                    last_accessed_at, access_count
             FROM memories
             WHERE content LIKE ?1 ESCAPE '\\'
             ORDER BY created_at DESC, id DESC LIMIT ?2",
        )?;
        let memories = statement
            .query_map(params![pattern, limit], memory_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(memories)
    }

    pub fn create_scope(&self, name: &str) -> Result<Scope> {
        let name = normalize_scope(name)?;
        let id = Uuid::now_v7();
        self.connection.execute(
            "INSERT INTO scopes (id, name, created_at) VALUES (?1, ?2, ?3)",
            params![id.to_string(), name, now_millis()?],
        )?;
        self.get_scope(id)?.ok_or(StorageError::Invalid {
            field: "scope",
            message: "scope was not created",
        })
    }

    pub fn get_scope(&self, id: Uuid) -> Result<Option<Scope>> {
        self.connection
            .query_row(
                "SELECT id, name, created_at FROM scopes WHERE id = ?1",
                [id.to_string()],
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

    pub fn delete_scope(&self, id: Uuid) -> Result<bool> {
        Ok(self
            .connection
            .execute("DELETE FROM scopes WHERE id = ?1", [id.to_string()])?
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

    pub fn attach_scopes(&mut self, memory_id: Uuid, scope_ids: &[Uuid]) -> Result<()> {
        self.with_transaction(|transaction| {
            for scope_id in scope_ids {
                transaction.execute(
                    "INSERT INTO memory_scopes (memory_id, scope_id) VALUES (?1, ?2)",
                    params![memory_id.to_string(), scope_id.to_string()],
                )?;
            }
            Ok(())
        })
    }

    pub fn detach_scope(&self, memory_id: Uuid, scope_id: Uuid) -> Result<bool> {
        Ok(self.connection.execute(
            "DELETE FROM memory_scopes WHERE memory_id = ?1 AND scope_id = ?2",
            params![memory_id.to_string(), scope_id.to_string()],
        )? == 1)
    }

    pub fn list_memory_scopes(&self, memory_id: Uuid) -> Result<Vec<Scope>> {
        let mut statement = self.connection.prepare(
            "SELECT s.id, s.name, s.created_at
             FROM scopes s
             JOIN memory_scopes ms ON ms.scope_id = s.id
             WHERE ms.memory_id = ?1 ORDER BY s.name",
        )?;
        let scopes = statement
            .query_map([memory_id.to_string()], scope_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(scopes)
    }

    pub fn create_entity(&self, kind: &str, name: &str, canonical_name: &str) -> Result<Entity> {
        validate_text("kind", kind)?;
        validate_text("name", name)?;
        validate_text("canonical_name", canonical_name)?;

        let id = Uuid::now_v7();
        let timestamp = now_millis()?;
        self.connection.execute(
            "INSERT INTO entities
             (id, kind, name, canonical_name, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![
                id.to_string(),
                kind.trim(),
                name.trim(),
                canonical_name.trim(),
                timestamp
            ],
        )?;
        self.get_entity(id)?.ok_or(StorageError::Invalid {
            field: "entity",
            message: "entity was not created",
        })
    }

    pub fn get_entity(&self, id: Uuid) -> Result<Option<Entity>> {
        self.connection
            .query_row(
                "SELECT id, kind, name, canonical_name, created_at, updated_at
                 FROM entities WHERE id = ?1",
                [id.to_string()],
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
        id: Uuid,
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
                id.to_string(),
                kind.trim(),
                name.trim(),
                canonical_name.trim(),
                now_millis()?
            ],
        )? == 1)
    }

    pub fn delete_entity(&self, id: Uuid) -> Result<bool> {
        Ok(self
            .connection
            .execute("DELETE FROM entities WHERE id = ?1", [id.to_string()])?
            == 1)
    }

    pub fn create_edge(
        &self,
        source_id: Uuid,
        relation: &str,
        target_id: Uuid,
        metadata: Option<&str>,
    ) -> Result<Edge> {
        validate_text("relation", relation)?;
        let id = Uuid::now_v7();
        self.connection.execute(
            "INSERT INTO edges (id, source_id, relation, target_id, created_at, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                id.to_string(),
                source_id.to_string(),
                relation.trim(),
                target_id.to_string(),
                now_millis()?,
                metadata
            ],
        )?;
        self.get_edge(id)?.ok_or(StorageError::Invalid {
            field: "edge",
            message: "edge was not created",
        })
    }

    pub fn get_edge(&self, id: Uuid) -> Result<Option<Edge>> {
        self.connection
            .query_row(
                "SELECT id, source_id, relation, target_id, created_at, metadata
                 FROM edges WHERE id = ?1",
                [id.to_string()],
                edge_from_row,
            )
            .optional()
            .map_err(StorageError::from)
    }

    pub fn update_edge(&self, id: Uuid, relation: &str, metadata: Option<&str>) -> Result<bool> {
        validate_text("relation", relation)?;
        Ok(self.connection.execute(
            "UPDATE edges SET relation = ?2, metadata = ?3 WHERE id = ?1",
            params![id.to_string(), relation.trim(), metadata],
        )? == 1)
    }

    pub fn delete_edge(&self, id: Uuid) -> Result<bool> {
        Ok(self
            .connection
            .execute("DELETE FROM edges WHERE id = ?1", [id.to_string()])?
            == 1)
    }

    pub fn list_outgoing_edges(&self, source_id: Uuid) -> Result<Vec<Edge>> {
        self.list_edges(
            "SELECT id, source_id, relation, target_id, created_at, metadata
             FROM edges WHERE source_id = ?1 ORDER BY created_at, id",
            source_id,
        )
    }

    pub fn list_incoming_edges(&self, target_id: Uuid) -> Result<Vec<Edge>> {
        self.list_edges(
            "SELECT id, source_id, relation, target_id, created_at, metadata
             FROM edges WHERE target_id = ?1 ORDER BY created_at, id",
            target_id,
        )
    }

    fn list_edges(&self, sql: &str, id: Uuid) -> Result<Vec<Edge>> {
        let mut statement = self.connection.prepare(sql)?;
        let edges = statement
            .query_map([id.to_string()], edge_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(edges)
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

fn migrate(connection: &mut Connection) -> Result<()> {
    let version = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version > SCHEMA_VERSION {
        return Err(StorageError::UnsupportedSchema(version));
    }
    if version == 0 {
        let transaction = connection.transaction()?;
        transaction.execute_batch(
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
                 memory_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
                 scope_id TEXT NOT NULL REFERENCES scopes(id) ON DELETE CASCADE,
                 PRIMARY KEY (memory_id, scope_id)
             );
             CREATE INDEX idx_memory_scopes_scope_id ON memory_scopes(scope_id);
             CREATE TABLE entities (
                 id TEXT PRIMARY KEY NOT NULL,
                 kind TEXT NOT NULL,
                 name TEXT NOT NULL,
                 canonical_name TEXT NOT NULL,
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL,
                 UNIQUE (kind, canonical_name)
             );
             CREATE TABLE edges (
                 id TEXT PRIMARY KEY NOT NULL,
                 source_id TEXT NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
                 relation TEXT NOT NULL,
                 target_id TEXT NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
                 created_at INTEGER NOT NULL,
                 metadata TEXT
             );
             CREATE INDEX idx_edges_source_id ON edges(source_id);
             CREATE INDEX idx_edges_target_id ON edges(target_id);
             CREATE INDEX idx_edges_relation ON edges(relation);
             PRAGMA user_version = 1;",
        )?;
        transaction.commit()?;
    }
    Ok(())
}

fn memory_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Memory> {
    Ok(Memory {
        id: parse_uuid(row.get::<_, String>(0)?)?,
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
        id: parse_uuid(row.get::<_, String>(0)?)?,
        name: row.get(1)?,
        created_at: row.get(2)?,
    })
}

fn entity_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Entity> {
    Ok(Entity {
        id: parse_uuid(row.get::<_, String>(0)?)?,
        kind: row.get(1)?,
        name: row.get(2)?,
        canonical_name: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

fn edge_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Edge> {
    Ok(Edge {
        id: parse_uuid(row.get::<_, String>(0)?)?,
        source_id: parse_uuid(row.get::<_, String>(1)?)?,
        relation: row.get(2)?,
        target_id: parse_uuid(row.get::<_, String>(3)?)?,
        created_at: row.get(4)?,
        metadata: row.get(5)?,
    })
}

fn parse_uuid(value: String) -> rusqlite::Result<Uuid> {
    Uuid::parse_str(&value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
    })
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

fn like_pattern(value: &str) -> Result<String> {
    validate_text("query", value)?;
    let escaped = value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    Ok(format!("%{escaped}%"))
}

fn normalize_scope(value: &str) -> Result<String> {
    validate_text("scope", value)?;
    Ok(value.trim().to_owned())
}

fn now_millis() -> Result<i64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| StorageError::Clock)
        .and_then(|duration| i64::try_from(duration.as_millis()).map_err(|_| StorageError::Clock))
}
