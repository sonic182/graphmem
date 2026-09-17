CREATE TABLE IF NOT EXISTS memories (
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
CREATE UNIQUE INDEX IF NOT EXISTS idx_edges_source_relation_target
    ON edges(source_id, relation, target_id);
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
END;
