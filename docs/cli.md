# CLI reference

`gmem` is the human-facing CLI over the same SQLite store the MCP server
(`gmem mcp`, see [docs/mcp.md](mcp.md)) uses. Every command opens the
database at `~/.graphmem` by default; set `GRAPHMEM_HOME` to point at a
different data directory (for example a separate dev store), and put
`config.toml` there to configure embeddings (`[embedding]`, see
[docs/mcp.md](mcp.md)) and retrieval tuning (`[retrieval]`, see the
[README](../README.md#configuration)).

Every command below opens its own connection and exits — there is no
long-running CLI process to keep in sync with disk state.

## Global options

| Flag | Default | Notes |
| --- | --- | --- |
| `--embedding-batch-size <n>` | `GRAPHMEM_EMBEDDING_BATCH_SIZE`, then `[embedding] batch_size`, then 1 on CPU / 16 on CUDA | Texts per embedding model call. Must be at least 1. Accepted before or after the subcommand, and applies to `gmem mcp` too. |

## `gmem remember <content>`

Store a new narrative memory.

| Flag | Default | Notes |
| --- | --- | --- |
| `--type <memory_type>` | `observation` | Free-text category, e.g. `decision`, `convention`, `constraint`. |
| `--importance <0.0-1.0>` | `0.0` | How costly it would be for a future task to miss this. |
| `--scope <scope>` (repeatable) | the server repo scope, or `global` outside a repo | Pass multiple times to attach several scopes. |

The CLI cannot attach entities or relations to a memory, and there is no CLI
equivalent of the `relate` tool — both are MCP-only (the `remember` tool's
`entities`/`relations` fields, and the `relate` tool).

When embeddings are enabled, the memory's vector is computed and stored in
the same transaction as the memory, loading the model on first use. If the
model cannot load or embed, the command exits non-zero and nothing is stored.

Prints `remembered: <id>`.

```sh
gmem remember "Use cargo nextest for integration tests" \
  --type convention --importance 0.7 --scope repo:/absolute/path/to/project
```

## `gmem list`

List memories, newest first, no ranking.

| Flag | Default |
| --- | --- |
| `--scope <scope>` | all scopes |
| `--limit <n>` | `50` |

Output: one line per memory, tab-separated `id`, `memory_type`, `content`.

## `gmem show <id>`

Print one memory's full detail (id, type, importance, timestamps, scopes,
content). Exits non-zero with `memory not found` if the id doesn't exist.

## `gmem search <query>`

Semantic recall (embeddings + graph context via Personalized PageRank). If
embeddings are disabled, or the model fails to load, it reports the failure
on stderr and falls back to SQLite FTS5 lexical ranking — the same fallback
the MCP `recall` tool uses. Unlike `recall`, the CLI has no flag to force
lexical search.

| Flag | Default | Notes |
| --- | --- | --- |
| `--scope <scope>` (repeatable) | every scope in the store | Omitted scopes search the whole store, not just the current repo + global (that repo-aware default is an MCP-only behavior — see [docs/mcp.md](mcp.md)). |
| `--limit <n>` | `10` | |

Output: one line per result, tab-separated `score`, `id`, `memory_type`,
`content`.

## `gmem graph <kind> <name>`

Inspect the unscoped entity graph around one entity (does not search
narrative memories).

| Flag | Default | Range |
| --- | --- | --- |
| `--direction <incoming\|outgoing\|both>` | `both` | |
| `--max-depth <n>` | `1` | 1-3 |
| `--limit <n>` | `25` | 1-100 |

Output: first line is the entity (`kind`, `name`, `canonical_name`), then one
line per hop per path (`depth`, `direction`, `relation`, `entity kind`,
`entity name`), paths separated by blank lines.

## `gmem forget <id>`

Permanently delete one memory. Prints `forgot: <id>`. Exits non-zero with
`memory not found` if the id doesn't exist.

## `gmem flush`

Delete **everything** — all memories, scopes, entities, and edges in the
active data directory. Requires `--yes`; without it, exits non-zero with
`refusing to flush; rerun with --yes`. There is no undo.

## `gmem scopes`

List every scope in the store: one line per scope, tab-separated `id`,
`name`.

## `gmem reembed`

Eagerly recompute every memory, entity, and edge embedding under the
currently configured model, across **every** scope in the store (not just
the current repo + global). This is the migration command for switching
embedding models.

**Why it's needed:** embeddings otherwise only get (re)computed lazily, as a
side effect of whatever `remember`/`relate`/`recall` happens to touch.
`recall`/`search` re-embeds every entity and edge on every call (both are
unscoped), but for memories it only visits the scopes it was asked about —
so after changing `model`, a memory in a `repo:` scope nobody queries stays
embedded under the old model indefinitely. Worse, `search`/`recall` treats a
failed embed as soft failure and silently falls back to lexical ranking, so
a broken model swap can go unnoticed. `gmem reembed` fails loudly instead
(see below) and reaches the whole store in one explicit pass.

**There is no vector conversion** — different models produce incompatible
vector spaces, so "migrate" always means *recompute*, never *convert*. Each
embedding row is looked up and stored keyed by `(model, revision)`
(`memory_id`/`entity_id`/`edge_id` is the primary key, `model`/`revision`
are plain columns updated in place — see `src/infrastructure/sqlite.rs`), so
switching models just overwrites the row for that id; nothing needs
deleting first.

**Batched**: records are embedded `--embedding-batch-size` at a time. If a
batch fails, its records are retried one by one so each failure is still
reported individually.

**Idempotent and safe to re-run**: every item is cache-checked against
`(model, revision)` before being recomputed, so re-running `gmem reembed`
after the store is already current costs one `SELECT` per row, not a
recompute.

**Fails loudly, on purpose** — unlike `search`, it does not fall back to
lexical ranking. If `[embedding] enabled = false` (or
`GRAPHMEM_EMBEDDINGS=off`), it exits non-zero with `embeddings are
disabled; set [embedding] enabled = true (or unset GRAPHMEM_EMBEDDINGS) to
reembed`. If the configured model fails to download or load, it exits
non-zero with that error instead of silently doing nothing.

**One failing record never blocks the rest of the store**: `reembed` still
visits every memory, entity, and edge even if some individual record fails
to embed. Each failure is printed with the record's kind and id, and the
command exits non-zero if any occurred, but every other record still gets
migrated in the same pass — re-run `gmem reembed` afterward and only the
still-failing records are retried (everything else is already current and
skipped per the idempotency check above).

**Typical migration**, after selecting the persistent model in `config.toml`
(the default is `sentence-transformers/msmarco-MiniLM-L6-cos-v5`):

```sh
gmem reembed
# reembedded 14 memories, 16 entities, 7 edges under the current embedding model
```

If some records fail:

```sh
gmem reembed
# reembedded 13 memories, 16 entities, 7 edges under the current embedding model
# 1 item(s) failed to reembed:
#   memory 42: model inference failed: ...
```

Note that `revision` in the cache key doesn't include the compute backend
(CPU/CUDA) — switching `backend` alone does not invalidate existing
embeddings (correctly: the resulting vector is expected to be
device-independent), so `gmem reembed` after only a `backend` change is a
near-instant no-op, not a recompute.

Not exposed as an MCP tool — this is a one-shot operator/maintenance action
taken after a config change, not something an MCP client needs mid-session.

## `gmem doctor`

Print the resolved database path and a health status line. Currently a
fixed `status: healthy` if the database opened at all — there is no deeper
check yet. Store counts are available via the `stats` MCP tool; there is no
CLI equivalent.

## `gmem mcp`

Run the MCP server over stdio (newline-delimited JSON-RPC 2.0). See
[docs/mcp.md](mcp.md) for the tool contract, and the `gmem-development`
skill's `scripts/mcp_smoke.py` for a dependency-free way to smoke-test it by
hand.
