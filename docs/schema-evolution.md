# Schema evolution

Graphmem is in beta (`0.1.x`). The SQLite schema lives in `migrations/` and is applied by the versioned migration runner in `src/infrastructure/migrations.rs`.

## How migrations work

- `schema_migrations(version INTEGER PRIMARY KEY, applied_at INTEGER)` records what has run.
- `MIGRATIONS` in `migrations.rs` is an ordered list of `(version, sql)` pairs; each script is embedded from `migrations/NNNN_*.sql` with `include_str!`.
- On open, the runner reads the current version (`MAX(version)`) and applies every migration newer than it, each in its own transaction that also inserts the version row. A failed migration rolls back atomically, and reopening resumes from the last fully applied version.
- Current state: version 1, `migrations/0001_initial.sql`, creating `memories`, `scopes`, `memory_scopes`, `entities`, `edges`, `memory_entities`, the embedding tables, and the `memories_fts` FTS5 index.

## Adding a migration

1. Add `migrations/0002_<name>.sql`.
2. Append `(2, include_str!("../../migrations/0002_<name>.sql"))` to `MIGRATIONS`.
3. Add a test for the upgrade path when the change is not purely additive.

Never edit an already-released migration; always add a new one.

## Compatibility expectations

- Prefer additive changes: new tables or nullable columns. Existing memories, scopes, entities, and edges stay valid.
- Embeddings are populated lazily on recall; missing vectors are not an error.
- Graph links are created only from explicit `remember` entities and relations, never inferred during an upgrade.
- `tests/storage.rs` covers that migrations apply once and stay stable across reopen.

## Breaking changes in beta

Beta may still make incompatible changes. Prefer a migration that transforms or rebuilds the affected tables; back up first if the data matters:

```sh
cp ~/.graphmem/memory.sqlite ~/.graphmem/memory.sqlite.bak
```

Deleting the database (`rm ~/.graphmem/memory.sqlite`, or the file under another `GRAPHMEM_HOME`) is the last resort, and loses the stored memories.

The versioned-migration mechanism above is the same one that carries the schema once it stabilizes, so no separate system needs to be introduced later.
