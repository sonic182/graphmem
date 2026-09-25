# Graphmem

[![CI](https://github.com/sonic182/graphmem/actions/workflows/ci.yml/badge.svg)](https://github.com/sonic182/graphmem/actions/workflows/ci.yml)

Graphmem gives coding agents a shared, local memory. Claude Code, Codex, OpenCode, pi, and other MCP clients can use the same store, so project decisions and context survive new sessions and switches between tools. It finds relevant memories even when a query uses different words.

Under the hood, Graphmem stores scoped notes and linked entities in SQLite. Recall combines local embeddings with graph context and Personalized PageRank — the non-trained retrieval approach of [HippoRAG 2](https://proceedings.mlr.press/v267/gutierrez25a.html) (Gutiérrez et al., ICML 2025). SQLite FTS5 provides lexical search when embeddings are disabled or for per-request comparison.

## Highlights

- Local SQLite storage; no hosted service required.
- Scoped memories (`global` or `repo:/absolute/path`) with scope isolation.
- Verified entities and directed relations attached atomically to memories.
- Local `sentence-transformers/msmarco-MiniLM-L6-cos-v5` embeddings through Candle.
- CUDA, CPU, or automatic backend selection.
- MCP server over stdio with `remember`, `recall`, `update`, `stats`, `relate`, `graph`, `inspect`, and `forget` tools.
- `recall` accepts `use_embeddings: false` to force lexical FTS5 ranking.

## Install

```sh
cargo install --locked --path .                                  # from a checkout
cargo install --locked --git https://github.com/sonic182/graphmem # latest from GitHub
```

This puts `gmem` in `~/.cargo/bin/gmem`. The editor plugins assume it is on your `PATH`; see [docs/plugins.md](docs/plugins.md). `git` must also be on your `PATH` for repository-scoped memory: `gmem` runs `git rev-parse --show-toplevel` to derive the current repository, and falls back to `global` when it cannot.

## Use it with your coding agent

The binary works with any MCP client, but the Claude Code, Codex, and OpenCode plugins also install a skill and lifecycle guidance, so your agent recalls relevant context before a task and records durable decisions after it — without being asked each time. Each plugin bundles its own MCP server config, so there is no separate `mcp add` step. After `cargo install`:

```sh
# Claude Code
claude plugin marketplace add sonic182/graphmem
claude plugin install graphmem@graphmem
```

```sh
# Codex
codex plugin marketplace add sonic182/graphmem
codex plugin add graphmem@graphmem
```

```sh
# OpenCode
opencode plugin graphmem@git+https://github.com/sonic182/graphmem.git#master --global
```

pi has no native MCP support and needs a one-time adapter; see [docs/plugins.md](docs/plugins.md) for pi and the full details.

## Build

Stable Rust is required. CPU builds need no extra feature:

```sh
cargo build
```

With CUDA support (and a working CUDA toolkit):

```sh
cargo build --features cuda
cargo build --release --features cuda
```

On GPUs older than Ampere (compute capability below 8.0, such as GTX 16xx and RTX 20xx), recent toolkits (CUDA 12.9 and 13.x) fail to build: `candle-kernels` 0.11 redefines `__hmax_nan`/`__hmin_nan`, which those headers now provide (`nvcc` fails in `src/compatibility.cuh`). This is tracked upstream in [huggingface/candle#3737](https://github.com/huggingface/candle/issues/3737). Make sure `nvcc` is on `PATH` (on Arch/Manjaro, `/opt/cuda/bin`).

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

The default data directory is `~/.graphmem`. Set `GRAPHMEM_HOME` to use a separate store, for example `~/.graphmem-dev`.

See [docs/cli.md](docs/cli.md) for the complete command reference, including `gmem reembed` (the embedding-model migration command).

## MCP

Configure an MCP client to launch:

```json
{
  "mcpServers": {
    "graphmem": {
      "command": "~/.cargo/bin/gmem",
      "args": ["mcp"]
    }
  }
}
```

Use the absolute path to the binary; `~/.cargo/bin/gmem` is where `cargo install` puts it.

`recall` uses embeddings by default. For a lexical comparison, pass:

```json
{
  "query": "What handles intermittent outages?",
  "use_embeddings": false,
  "limit": 10
}
```

See [docs/mcp.md](docs/mcp.md) for the complete tool contract.

## Configuration

Configuration is optional — the values below are the built-in defaults and Graphmem runs fine without a config file. To change any of them, create `~/.graphmem/config.toml` (or `$GRAPHMEM_HOME/config.toml`):

```toml
[embedding]
enabled = true
backend = "auto"       # auto, cpu, or cuda
model = "sentence-transformers/msmarco-MiniLM-L6-cos-v5"
revision = "main"
cache_dir = "/home/user/.graphmem/models"
# batch_size = 16      # texts per model call; default 1 on CPU, 16 on CUDA

[retrieval]
seed_top_k = 20            # memories kept as PageRank seeds
seed_temperature = 0.05    # lower sharpens the gap between seeds
memory_seed_weight = 0.5   # share of seed mass for memories vs. the graph
entity_anchor_weight = 0.2 # pull toward entities named in the query
damping = 0.5

[runtime]
worker_threads = 4
```

`backend = "auto"` selects CUDA when available and otherwise uses CPU. `GRAPHMEM_EMBEDDINGS=off` disables embeddings globally. The model is downloaded and loaded on first use (the first `remember`, `relate`, recall, or `reembed`) and cached locally. `remember` stores its embeddings in the same transaction, so it fails and stores nothing if the model cannot load or embed. `batch_size` also reads `GRAPHMEM_EMBEDDING_BATCH_SIZE` and the `--embedding-batch-size` flag. Under the same precedence (env > flag > file), the environment wins over the flag and the flag wins over `config.toml`, for one `gmem` run including `gmem mcp`.

Every `[retrieval]` key also reads a `GRAPHMEM_RETRIEVAL_*` environment variable and a matching `--retrieval-*` flag (`--retrieval-damping`, etc.), so values can be swept without editing the file. Resolution is per field: **environment variable > command-line flag > `config.toml` > built-in default**. `gmem mcp` accepts the same flags. `seed_temperature` is the one that matters most: raising it flattens ranking toward returning the whole store.

Logs are appended to `~/.graphmem/logs/graphmem.log` (or the corresponding `GRAPHMEM_HOME` directory):

```sh
tail -f ~/.graphmem/logs/graphmem.log
```

## Development

```sh
just verify
```

This runs formatting checks, compilation, Clippy, and the test suite. More background is available in [docs/design.md](docs/design.md) and [docs/schema-evolution.md](docs/schema-evolution.md).

## License

MIT — see [LICENSE](LICENSE).
