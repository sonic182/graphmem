# Schema evolution

Graphmem is currently `0.0.1`. The SQLite schema in
`src/infrastructure/sqlite.rs` is always the latest schema known by the code;
there is no schema version table and no automatic migration system yet.

During this alpha stage, an incompatible schema change may require deleting
the local database and letting graphmem recreate it:

```bash
rm ~/.graphmem/memory.sqlite
```

For another data directory, remove its `memory.sqlite` file instead. Before
doing so, make a copy if the local memories matter.

When the schema becomes stable enough to preserve user data across releases,
introduce explicit versioned migrations, backup/restore guidance, and tests
for each supported upgrade path.
