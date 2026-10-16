CREATE TABLE IF NOT EXISTS recall_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    at INTEGER NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('search', 'inspect')),
    source TEXT NOT NULL CHECK (source IN ('cli', 'mcp')),
    query TEXT,
    scopes TEXT NOT NULL,
    requested_limit INTEGER,
    returned INTEGER NOT NULL CHECK (returned >= 0)
);
CREATE INDEX IF NOT EXISTS idx_recall_events_at ON recall_events(at);
CREATE TABLE IF NOT EXISTS recall_results (
    event_id INTEGER NOT NULL REFERENCES recall_events(id) ON DELETE CASCADE,
    memory_id INTEGER NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    score REAL,
    PRIMARY KEY (event_id, memory_id)
);
CREATE INDEX IF NOT EXISTS idx_recall_results_memory_id ON recall_results(memory_id);
