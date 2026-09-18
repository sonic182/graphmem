# Graphmem design

Local, single-binary memory for coding agents.

**Language** Rust · **Storage** embedded SQLite · **Transport** MCP over stdio · **Clients** Claude Code, Codex, pi · **Data** `~/.graphmem` (or `$GRAPHMEM_HOME`).

## Purpose

Coding agents are strong within one session but lose context between them: architectural decisions, conventions, debugging discoveries, pitfalls, and how components relate. Graphmem keeps that context on disk and makes it retrievable from any MCP client and from a human CLI.

The bar is a tool that feels like `git`, `rg`, or `sqlite3`:

```text
install one binary → register it as an MCP server → memory persists
```

No daemon, no external database, no server to run.

## Non-goals

Not a general-purpose graph or vector database, not a hosted service, not a multi-user server, not an autonomous knowledge-extraction platform, and not a replacement for Git or project documentation. The priority is a tiny, reliable local memory layer.

## Architecture

```text
Claude Code / Codex / pi / human CLI
                │  MCP stdio or CLI
                ▼
          gmem (single Rust binary)
                │  embedded
                ▼
          SQLite + FTS5
          memories · scopes · entities · relations
```

The process is launched by the client and exits with it. There is no daemon.

## Storage

SQLite (`rusqlite`, bundled) is the store: one binary, transactions, schema migrations, indexes, recursive CTEs, FTS5, trivial backup, and no server. The expected dataset is small, so a graph database earns nothing — relations live in ordinary tables and traversal uses recursive CTEs. A key/value store such as RocksDB was rejected because it would mean rebuilding the filtering, search, and relationship indexes SQLite already provides.

Writes are transactional: a `remember` that fails to embed stores nothing.

## Data model

**Memory** — one durable piece of remembered information: `content`, `memory_type`, `importance`, `created_at`, `updated_at`, `last_accessed_at`, `access_count`. `memory_type` is a free string (e.g. `decision`, `convention`, `observation`); there is no fixed ontology.

**Scope** — `global` or `repo:/absolute/path`. A memory may have several scopes. `recall` filters by scope before ranking, so one repository's memories never leak into another's results. When the caller omits scopes, the server uses its repository (derived with `git rev-parse --show-toplevel`) plus `global`, repository first.

**Entity** — a named thing a memory refers to (`kind` + `name` + normalized form). Kinds are strings, not an enum.

**Relation** — a directed, labelled edge between two entities, with optional metadata. A memory links to the entities and relations it mentions.

## Retrieval

`recall` seeds from the query's semantic embedding and text matches, then runs Personalized PageRank over the memory↔entity graph to reach memories the query did not literally match. This is the non-trained [HippoRAG 2](https://proceedings.mlr.press/v267/gutierrez25a.html) approach. Scope filtering happens before ranking.

SQLite FTS5 (BM25, then importance and recency tie-breakers) is the lexical fallback and a per-call comparison mode via `use_embeddings: false`.

Embeddings run locally through Candle, default `sentence-transformers/msmarco-MiniLM-L6-cos-v5`, downloaded on first use and cached under the data directory. `GRAPHMEM_EMBEDDINGS=off` disables them, leaving FTS5 ranking. A memory's vectors are stored in the same transaction as the memory, so a model failure stores nothing and never leaves a half-indexed row.

## MCP interface

Seven tools, kept small on purpose: `remember`, `recall`, `stats`, `relate`, `graph`, `inspect`, `forget`. The agent works with memory semantics, never with `create_node`/`query_sql`-style database operations. See [mcp.md](mcp.md) for the contract.

## CLI

`gmem` also has a human-facing CLI so the store stays inspectable without an LLM:

```sh
gmem remember "…"   gmem list   gmem show <id>   gmem search "…"
gmem graph <kind> <name>   gmem forget <id>   gmem flush
gmem scopes   gmem reembed   gmem doctor   gmem mcp
```

See [cli.md](cli.md).

## Logging and stdout

MCP speaks newline-delimited JSON-RPC on stdout, so application logs must never touch it. Logs go to stderr and to `~/.graphmem/logs/graphmem.log` (following `GRAPHMEM_HOME`).

## Privacy

Local-first: no telemetry, no network requests beyond the one-time model download, no hosted API, no automatic upload. Secrets are not intentionally stored.

## Testing

Behavior over implementation. Integration tests cover SQLite and the CLI/MCP boundary; unit tests are for pure domain logic such as ranking. The suite exercises persistence across process restarts, scope isolation, graph traversal (including cycles), and that logs never contaminate stdout.

## Principles

- Keep the MCP surface small; add a tool only when a memory semantic needs one.
- SQLite is an implementation detail; clients never see the schema.
- Deterministic operations first; no autonomous extraction without evidence it helps.
- Optimize retrieval quality, not graph complexity — the graph serves recall, it is not the product.
- Everything a user stores stays inspectable, exportable, and deletable.
- Preserve the single-binary property when adding dependencies.
