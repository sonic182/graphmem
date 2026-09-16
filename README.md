# Graphmem

Graphmem is a local memory service for agents and developer tools. It stores
scoped narrative memories alongside an entity graph, then combines semantic
embeddings, graph context, and Personalized PageRank for recall. SQLite FTS5
remains available as a lexical fallback and as a per-request comparison mode.

## Highlights

- Local SQLite storage; no hosted service required.
- Scoped memories (`global` or `repo:/absolute/path`) with scope isolation.
- Verified entities and directed relations attached atomically to memories.
- Local `sentence-transformers/msmarco-distilbert-cos-v5` embeddings through Candle.
- CUDA, CPU, or automatic backend selection.
- MCP server over stdio with `remember`, `recall`, `stats`, `relate`, `graph`,
  `inspect`, and `forget` tools.
- `recall` accepts `use_embeddings: false` to force lexical FTS5 ranking.

## Build

Stable Rust is required. CPU builds need no extra feature:

```sh
cargo build
```

With CUDA support (and a working CUDA toolkit):

```sh
./debug_build.sh       # cargo build --features cuda
./build.sh              # cargo build --release --features cuda
```

The binary is `target/debug/gmem` or `target/release/gmem`.

## CLI

```sh
gmem remember "Use nextest for integration tests" \
  --type convention --scope repo:/absolute/path/to/project
gmem search "integration tests"
gmem graph component api --direction both --max-depth 2
gmem reembed
gmem mcp
```

The default data directory is `~/.graphmem`. Set `GRAPHMEM_HOME` to use a
separate store, for example `~/.graphmem-dev`.

See [docs/cli.md](docs/cli.md) for the complete command reference, including
`gmem reembed` (the embedding-model migration command).

## MCP

Configure an MCP client to launch:

```json
{
  "mcpServers": {
    "graphmem": {
      "command": "/absolute/path/to/gmem",
      "args": ["mcp"]
    }
  }
}
```

`recall` uses embeddings by default. For a lexical comparison, pass:

```json
{
  "query": "What handles intermittent outages?",
  "use_embeddings": false,
  "limit": 10
}
```

See [docs/mcp.md](docs/mcp.md) for the complete tool contract.

## Codex plugin

From a local Graphmem checkout, install the binary and plugin with one command:

```sh
just install-codex-plugin
```

For a CUDA build, use:

```sh
just install-codex-plugin true
```

The recipe installs `gmem` and registers its MCP server only when `gmem` is
not already configured; the optional `true` adds Cargo's `cuda` feature for that new
install. It then adds the checkout as a Codex marketplace and installs
`graphmem`. An existing `gmem` configuration is left unchanged. Open `/hooks`
in Codex, review and trust the two Graphmem hooks, then start a new thread.

To install the plugin from GitHub, install `gmem` on your `PATH` first, then:

```sh
codex plugin marketplace add sonic182/graphmem
codex plugin add graphmem@graphmem
```

The plugin uses Codex's `gmem mcp` server, so semantic calls reuse the same
in-memory embedding model. The lifecycle hooks require `node` on your `PATH`;
without it, the MCP server still works but Codex does not receive the
recall/store guidance.

## Configuration

Create `~/.graphmem/config.toml` (or `$GRAPHMEM_HOME/config.toml`):

```toml
[embedding]
enabled = true
backend = "auto"       # auto, cpu, or cuda
model = "sentence-transformers/msmarco-distilbert-cos-v5"
revision = "main"
cache_dir = "/home/user/.graphmem/models"

[retrieval]
seed_top_k = 20            # memories kept as PageRank seeds
seed_temperature = 0.05    # lower sharpens the gap between seeds
memory_seed_weight = 0.5   # share of seed mass for memories vs. the graph
entity_anchor_weight = 0.2 # pull toward entities named in the query
damping = 0.5

[runtime]
worker_threads = 4
```

`backend = "auto"` selects CUDA when available and otherwise uses CPU.
`GRAPHMEM_EMBEDDINGS=off` disables embeddings globally. The model is downloaded
on first enabled recall and cached locally.

Every `[retrieval]` key also reads a `GRAPHMEM_RETRIEVAL_*` environment variable,
so values can be swept without editing the file. `seed_temperature` is the one
that matters most: raising it flattens ranking toward returning the whole store.

Logs are appended to `~/.graphmem/logs/graphmem.log` (or the corresponding
`GRAPHMEM_HOME` directory):

```sh
tail -f ~/.graphmem/logs/graphmem.log
```

## Development

```sh
just verify
```

This runs formatting checks, compilation, Clippy, and the test suite. More
background is available in [Project_doc.md](Project_doc.md) and
[docs/schema-evolution.md](docs/schema-evolution.md).
