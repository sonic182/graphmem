# Build and configuration

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

With the code navigation tools (`gmem code`, and the `code_outline`,
`code_imports`, `find_symbol`, and `code_diff` MCP tools; release binaries include them).
They outline Rust, Go, Zig, C, C++, Python, JavaScript/TypeScript,
Elixir/Phoenix templates, Ruby, PHP, Racket, SQL, Bash, CSS, SCSS, and HTML
`<script>`/`<style>`, and add
about 20 MB to the binary:

```sh
cargo build --features code
```

On GPUs older than Ampere (compute capability below 8.0, such as GTX 16xx and RTX 20xx), recent toolkits (CUDA 12.9 and 13.x) fail to build: `candle-kernels` 0.11 redefines `__hmax_nan`/`__hmin_nan`, which those headers now provide (`nvcc` fails in `src/compatibility.cuh`). This is tracked upstream in [huggingface/candle#3737](https://github.com/huggingface/candle/issues/3737). Make sure `nvcc` is on `PATH` (on Arch/Manjaro, `/opt/cuda/bin`).

For full performance on the machine that will run it, build for the local CPU so the compiler can use its newest instructions (AVX2, AVX-512, and so on). This speeds up CPU embedding inference in particular. Drop `cuda` from the feature list if you have no CUDA toolkit:

```sh
RUSTFLAGS="-C target-cpu=native" cargo build --release --features cuda,code
```

A `target-cpu=native` binary may crash with an illegal instruction on older or different CPUs. Build it on the machine that runs it, and do not ship it. Release artifacts target a generic CPU.

The binary is `target/debug/gmem` or `target/release/gmem`.

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

[code]
enabled = true         # only in builds with --features code
max_files = 20000      # source files indexed per checkout
index_threads = "auto" # or a number of parsing threads
```

`[code] enabled = false` (or `GRAPHMEM_CODE=off`) hides `gmem code` and the code MCP tools in a binary built with them. Raise `max_files` (or set `GRAPHMEM_CODE_MAX_FILES`) for a larger checkout; the first index then takes longer. `index_threads` (or `GRAPHMEM_CODE_INDEX_THREADS`) sets how many threads parse files while indexing. `"auto"` uses the CPUs the process may run on, honoring CPU affinity and cgroup quotas such as a container's `--cpus` limit. The threads exist only while changed files are being parsed.

`backend = "auto"` selects CUDA when available and otherwise uses CPU. `GRAPHMEM_EMBEDDINGS=off` disables embeddings globally. The model is downloaded and loaded on first use (the first `remember`, `relate`, recall, or `reembed`) and cached locally. `remember` stores its embeddings in the same transaction, so it fails and stores nothing if the model cannot load or embed. `batch_size` also reads `GRAPHMEM_EMBEDDING_BATCH_SIZE` and the `--embedding-batch-size` flag. Under the same precedence (env > flag > file), the environment wins over the flag and the flag wins over `config.toml`, for one `gmem` run including `gmem mcp`.

Every `[retrieval]` key also reads a `GRAPHMEM_RETRIEVAL_*` environment variable and a matching `--retrieval-*` flag (`--retrieval-damping`, etc.), so values can be swept without editing the file. Resolution is per field: **environment variable > command-line flag > `config.toml` > built-in default**. `gmem mcp` accepts the same flags. `seed_temperature` is the one that matters most: raising it flattens ranking toward returning the whole store.

Logs are appended to `~/.graphmem/logs/graphmem.log` (or the corresponding `GRAPHMEM_HOME` directory):

```sh
tail -f ~/.graphmem/logs/graphmem.log
```
