---
name: gmem-development
description: Guides development in the Graphmem/gmem Rust repository — its layered hexagonal architecture, the check/test/lint/fmt feedback loop, and testing conventions. Use when writing, reviewing, or debugging gmem code, or when the MCP stdio server (`gmem mcp`) needs to be smoke-tested by sending raw JSON-RPC requests over stdin/stdout.
compatibility: Rust repository; the bundled smoke-test script requires Python 3.9+, a Unix-like OS (uses select() on pipes), and a built `gmem` binary (`cargo build`).
---

# Developing Graphmem (gmem)

Graphmem is a local memory service (scoped narrative memories + an unscoped
entity graph) exposed as a CLI (`gmem ...`) and an MCP stdio server
(`gmem mcp`). Storage is SQLite; semantic recall uses local Candle embeddings
plus Personalized PageRank over the entity graph, with SQLite FTS5 as a
lexical fallback/comparison mode. Background: `CLAUDE.md`, `README.md`,
`Project_doc.md`, `docs/mcp.md`.

## Architecture

A small layered/hexagonal split — know which file a change belongs in before
writing it:

- `src/domain.rs` — domain data and invariants (PageRank, matching, etc.).
  Must not depend on Clap, rusqlite, or transport code. Unit-test this layer.
- `src/infrastructure/` — external adapters: `sqlite.rs` (schema + storage),
  `embedding.rs` (Candle model loading/inference), `config.rs`,
  `logging.rs`, `repository.rs`.
- `src/application.rs` — use cases that coordinate domain + infrastructure
  (e.g. `semantic_results`, `memory_vector`). Generic over the
  `EmbeddingModel` trait rather than a concrete embedder.
- `src/cli.rs` — human CLI: argument parsing, output formatting, delegates
  to `application.rs`.
- `src/mcp.rs` — the MCP stdio server: tool schemas and the same
  `application.rs` use cases behind JSON-RPC.
- `src/main.rs` — composition root (concrete adapters, process exit codes).

**Load-bearing convention:** don't add ports, repository traits, factories,
events, or DI machinery until a second adapter or a real testing boundary
requires one. When it *does* show up, the minimal fix is usually a plain enum
matched once at the construction boundary — e.g. `Embedder`'s `Backbone` enum
(`Qwen3` / `DistilBert`) in `src/infrastructure/embedding.rs`, added only once
a second model architecture was actually needed, not before.

## Feedback loop

- `rg` for text search, `ast-grep` for syntax-aware search — both preferred
  over plain `grep`/manual reading when available.
- `just check` / `just test` / `just lint` while iterating (package-scoped).
- `just fmt` after edits.
- `just verify` (fmt-check + check + lint + test) before handing off changes.
- `just ra` for supplemental rust-analyzer diagnostics.
- Keep MCP protocol frames on stdout; application logs belong on stderr /
  `$GRAPHMEM_HOME/logs/graphmem.log` (see below) — never `println!` into a
  code path `mcp.rs` can reach, it would corrupt the protocol stream.

## Testing conventions

Prefer integration tests for SQLite and CLI/MCP behavior; add unit tests only
for pure domain logic. Existing examples to follow:

- `tests/storage.rs`, `tests/search.rs` — SQLite behavior end-to-end.
- `tests/cli.rs` — spawns the built binary as a subprocess.
- `tests/mcp.rs` — spawns `gmem mcp` and drives it over real stdio JSON-RPC;
  the canonical reference for the wire protocol (see below).
- `src/domain.rs` / `src/infrastructure/embedding.rs` unit tests — small,
  pure-function tests (PageRank math, weight-name mapping) that don't need a
  process or a database.

Real embedding-model tests are avoided in the automated suite (network +
multi-hundred-MB download); tests that touch `Embedder` disable embeddings
via `config.toml` (`[embedding]\nenabled = false`) or fake the
`EmbeddingModel` trait. Do the same in new tests unless you're specifically
testing embedding behavior — then smoke-test it manually (next section)
rather than adding it to `just test`.

## MCP stdio protocol

`gmem mcp` speaks **newline-delimited JSON-RPC 2.0** — one JSON object per
line, both directions, no Content-Length framing (unlike LSP). Match
responses to requests by `id`; the server may interleave responses out of
order. stdout carries *only* protocol frames; diagnostics go to stderr and to
`$GRAPHMEM_HOME/logs/graphmem.log` (`~/.graphmem/logs/graphmem.log` by
default). `GRAPHMEM_HOME` selects the data directory — always point it at a
scratch directory for smoke testing so you don't touch real memories.

Minimal exchange (see `tests/mcp.rs` for the full reference):

```jsonc
// -> stdin
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}
// <- stdout
{"jsonrpc":"2.0","id":1,"result":{"serverInfo":{"name":"gmem",...},"instructions":"..."}}

// -> stdin
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"remember","arguments":{"content":"..."}}}
// <- stdout
{"jsonrpc":"2.0","id":2,"result":{"structuredContent":{"id":1,"scopes":["global"]},...}}
```

The eight tools (`remember`, `recall`, `update`, `stats`, `relate`, `graph`,
`inspect`, `forget`) and their exact argument/response shapes are documented in
`docs/mcp.md` and exercised in `tests/mcp.rs`.

### Smoke-testing it

Use `scripts/mcp_smoke.py` — a dependency-free Python client that speaks this
protocol directly, instead of hand-rolling a bidirectional pipe in the
shell:

```sh
# full remember/recall/inspect/forget lifecycle, lexical-only (fast, no model download)
.agents/skills/gmem-development/scripts/mcp_smoke.py

# same, but exercises the real embedding model (downloads it on first run)
.agents/skills/gmem-development/scripts/mcp_smoke.py --embeddings

# print every request/response while running the smoke test
.agents/skills/gmem-development/scripts/mcp_smoke.py -v

# one ad-hoc tool call after initialize (auto-initializes for you)
.agents/skills/gmem-development/scripts/mcp_smoke.py call recall '{"query":"embedding model","use_embeddings":false}'

# one ad-hoc raw JSON-RPC method (e.g. to probe tools/list shape)
.agents/skills/gmem-development/scripts/mcp_smoke.py raw tools/list '{}'

# point at a real/dev store instead of a throwaway temp dir
.agents/skills/gmem-development/scripts/mcp_smoke.py --home ~/.graphmem-dev call stats '{}'
```

Defaults: auto-detects the binary (`target/debug/gmem`, then
`target/release/gmem`, then `PATH`; override with `--bin`), creates a scratch
`GRAPHMEM_HOME` in a temp dir and deletes it on exit (`--keep` to retain it,
`--home DIR` to use a specific one instead), and disables embeddings unless
`--embeddings` is passed (mirrors the `config.toml` pattern `tests/mcp.rs`
uses, so the default run is fast and network-free). Exits non-zero with a
message on protocol timeout, a closed pipe, or an unexpected response shape.

Run it after any change to `src/mcp.rs`, tool schemas, or `application.rs`
use cases the MCP tools call through — it's a faster way to see the raw
protocol response than reading `tests/mcp.rs` assertions.
