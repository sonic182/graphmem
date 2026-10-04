# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.9.2] - 2026-10-04

### Changed

- The npm package now includes the Markdown documentation while excluding the
  large banner asset; README and Pi gallery images use the stable GitHub URL.
- The Pi package listing has a descriptive summary, gallery image, and keywords
  highlighting coding-agent memory, semantic search, and the knowledge graph.
- The README highlights the Pi npm install command, and Pi setup instructions use
  its built-in MCP configuration instead of the obsolete adapter workflow.

## [0.9.1] - 2026-10-04

### Added

- npm installation via `npm install --global @sonic182/graphmem`, with a `gmem` launcher
  that reuses an existing working binary or downloads and verifies the matching
  GitHub Release binary. `gmem-install` retries binary installation when npm
  lifecycle scripts are disabled.
- npm package tests and a package file allowlist; installation instructions now
  cover npm alongside manual binary and Cargo installation.

## [0.9.0] - 2026-10-02

### Added

- Racket (`.rkt`, `.rktl`) is indexed by the code tools: modules, structs,
  functions, macros, constants, and `require` imports.
- A real-repository [code navigation validation baseline](docs/roadmap/code-navigation-validation.md)
  for Phoenix/Elixir and Python, covering indexing and query timings,
  definition accuracy, output size, and the known Memoize extraction gap.

### Fixed

- Code indexes use versioned filenames (`code-v{N}.sqlite`), so binaries with
  different schema/extractor versions no longer wipe each other's cache.
  Older caches are left untouched.
- Racket phase-shifted `require` forms (`for-syntax`, `for-template`,
  `for-label`, and `for-meta`) extract every nested module spec.
  `for-meta` skips its phase argument, including `#f`. Imports retain their
  individual source ranges and remain siblings, avoiding missing imports in
  depth-limited outlines and false parent names in symbol lookups.

## [0.8.0] - 2026-10-01

### Added

- `gmem code imports <FILE>` and the MCP `code_imports` tool list declared
  imports with their source ranges without resolving them to files. Ruby
  `require_relative`/`load`, PHP `require`/`include`, Bash `source`, Rust
  `extern crate`, and JavaScript/TypeScript `require()`, `import()`,
  `export ... from`, and `import x = require()` are listed too.
- `gmem code outline <FILE> --depth N` and the MCP `code_outline` `depth`
  argument limit how deeply nested symbols are listed; 0 returns top-level
  symbols only. Pagination counts only the symbols within that depth.
- `gmem code diff <BASE> [HEAD]` and the MCP `code_diff` tool list the symbols
  added, removed, or modified between the merge base of two Git revisions and
  the head, per changed file, by outlining both versions in memory. Edits
  inside a nested symbol are reported on that symbol only; renamed symbols
  appear as removed plus added; unsupported and binary files are listed as
  skipped. The MCP tool answers in compact plain text rather than JSON (about
  7× smaller than `git diff` on real repositories) and reports at most `limit`
  changed files (default 200).

### Changed

- `export ... from` is now outlined as an `import` instead of a `module`, so
  `find_symbol` with `kind: "module"` no longer returns re-exports. The new
  import forms (for example Bash `source`) also appear as `import` symbols in
  `code_outline` and `find_symbol`. The code index is rebuilt once on upgrade to
  pick up the new symbols.
- The MCP `code_outline`, `code_imports`, and `find_symbol` tools answer in
  plain text, like `code_diff` and the CLI, instead of JSON. Nesting is
  shown by indentation, and signatures only when they add to the name.
  Columns, outline indexes, and `fresh` markers are gone. An outline of a
  200-symbol file shrinks from 69 KB to 9 KB on the wire.
- `gmem code find` prints `path:start-end<TAB>kind name`, then the parent,
  freshness, and signature only when they apply. Its `N of TOTAL matches`
  and `truncated` notes now go to stdout. `gmem code outline` adds
  signatures.
- The session-start context tells the agent to load the
  `graphmem-code-analysis` skill before its first code lookup, and the skill
  now also triggers on addressing PR review comments and reading another
  checkout.

### Fixed

- The MCP `find_symbol` and `code_outline` tools refresh the code index on the
  blocking thread pool instead of a Tokio worker. A long first index, or calls
  queued behind it, no longer stalls the memory tools.

## [0.7.1] - 2026-09-30

### Fixed

- Windows MSVC builds with the `code` feature use the upstream SCSS grammar
  build fix, which selects a compiler-compatible warning flag. Other platforms
  retain the crates.io dependency. CI now checks this feature on Windows before
  release builds.

## [0.7.0] - 2026-09-30

### Added

- The code index outlines CSS and SCSS: rule sets (named by their selector
  list), `@media`, `@supports`, `@keyframes`, SCSS `@mixin` and `@function`,
  `--custom` properties and `$variables`, and `@import`, `@use`, and `@forward`
  as imports. `.min.css` files are skipped like `.min.js`.
- HTML `<style>` and HEEx (including `~H`) `<script>` and `<style>` bodies are
  outlined at their position in the file.
- `graphmem-code-analysis` skill in the Claude Code, Codex, OpenCode, and pi
  plugins. It covers which of `find_symbol` and `code_outline` fits a task,
  how to read `coverage`, `truncated`, and `freshness`, and when to fall back
  to `rg` or `ast-grep` for call sites and references. The session guidance
  points agents to it.

## [0.6.0] - 2026-09-30

### Added

- `gmem reembed` now reports model loading and per-type processing progress on
  stderr during long migrations.
- Optional `code` Cargo feature (on in release builds): `gmem code
  index|outline|find` and the `code_outline` and `find_symbol` MCP tools
  outline Rust, Go, Zig, C, C++, Python, JavaScript/JSX,
  TypeScript/TSX, Elixir (with `~H`), HEEx, EEx, Ruby, PHP, SQL, Bash, and
  HTML `<script>` from a separate, rebuildable index per Git checkout.
  `[code] enabled = false` or `GRAPHMEM_CODE=off` hides them, and
  `[code] max_files` (or `GRAPHMEM_CODE_MAX_FILES`, default 20,000) bounds the
  files indexed per checkout. Indexing parses files in parallel, on
  `[code] index_threads` threads (`"auto"` or a number; also
  `GRAPHMEM_CODE_INDEX_THREADS`).
  `find_symbol` refreshes the index before searching, and reports `total` and
  `truncated`. It also matches last name segments (`ConsentLive`,
  `public.users`), and leaves out imports and template usages unless `kind`
  asks for them. `gmem mcp` keeps serving the memory tools when the code index
  or its configuration is broken, and a corrupt `code.sqlite` is recreated.
- Plugin session and subagent guidance points agents to `find_symbol` and
  `code_outline` when those tools are available.

## [0.5.0] - 2026-09-29

### Added

- `gmem version` prints the installed binary version.
- Keyboard-driven `gmem tui` to browse memories and graph nodes, filter
  memories, view colored Markdown details, edit memory text with an external
  editor, and delete with confirmation.
- ModernBERT embedding checkpoints with CLS pooling, including the
  multilingual `ibm-granite/granite-embedding-97m-multilingual-r2`. Set it as
  `[embedding] model` and run `gmem reembed` to migrate existing vectors.

### Changed

- Embedding inputs are capped at 2,048 tokens (or the checkpoint's lower
  `max_position_embeddings`) to bound long-context attention memory. Truncation
  continues to be reported in warnings.
- ModernBERT uses F32 inference, including on CUDA, to support GPUs without
  BF16 kernels.
- Retrieval evaluations now reuse cached vectors across graph runs and sweep
  configurations over one ingested store, avoiding repeated embedding and
  ingestion work.

### Fixed

- MCP repository scopes now use Git's common directory so linked worktrees share
  memories, including when the repository is bare. Explicit checkout paths
  resolve to the shared scope; separate clones remain isolated.
- ModernBERT now rejects checkpoints with unsupported bias flags or pooling,
  and invalid attention dimensions return errors instead of panicking. Layer
  normalization load errors are no longer silently ignored.
- Graph paths now include self-relations in both directions without following
  them repeatedly.

## [0.4.0] - 2026-09-28

### Added

- CPU-only prebuilt `gmem` binaries for Linux x86-64, macOS Intel and Apple
  Silicon, and Windows x86-64. Version tags publish archives and SHA-256
  checksums to GitHub Releases; manual workflow runs build artifacts without
  publishing a release.

## [0.3.2] - 2026-09-28

### Fixed

- `recall`/`search` and `inspect`/`show` now update `last_accessed_at` and
  increment `access_count` for each memory returned. List commands and internal
  reads do not count as accesses.

## [0.3.1] - 2026-09-22

### Fixed

- MCP responses now always include `warnings` as an array, including `[]` when
  no embedding input was truncated. This keeps emitted `remember`, `recall`,
  and `relate` responses consistent with their required output schemas.

## [0.3.0] - 2026-09-22

### Added

- `warnings` field on the `remember`, `recall`, and `relate` responses, set
  when an input was longer than the embedding model's token limit and was
  truncated before embedding. Truncation was silent before, so an agent could
  store a long memory and never learn that only its first tokens affect
  ranking. The field is omitted when nothing was truncated, and each truncated
  document is also logged as a warning.
- `gmem://embedding` MCP resource, returning the active model, revision, and
  the exact `max_tokens` read from the checkpoint's `config.json`. Only that
  file is fetched, from the local cache once the model has been used, and
  `max_tokens` is null when embeddings are disabled.
- The `initialize` instructions now name the active embedding model, or report
  that embeddings are disabled and recall is lexical.

## [0.2.0] - 2026-09-22

### Added

- `update` MCP tool, which revises one memory in place by `id`. The `content`,
  `memory_type`, and `importance` fields you pass replace the stored ones and
  the fields you omit are kept, so a decision that changed no longer has to be
  stored as a second, competing memory. Scopes, entities, and relations are
  left untouched.
- Re-embedding of updated content in the same transaction as the update, so a
  model that cannot load or embed leaves the memory unchanged, matching how
  `remember` stores nothing on an embedding failure.
- `memory_type` filter on `recall`, applied before ranking and matched without
  regard to case. A filtered-out memory neither seeds nor propagates graph
  rank, and the filter works on both the semantic and the lexical path.
- Scope enforcement on the id-addressed tools. `inspect`, `update`, and
  `forget` now reach only a memory that has no scopes, a `global` memory, or
  one in the server's own repository scope, and report `memory not found`
  otherwise. Ids are allocated across the whole store, so before this an agent
  in one repository could read, overwrite, or delete another repository's
  memory by guessing an id, despite recall's scope isolation. The three tools
  take the same optional `scopes` argument as `remember` and `recall`, so a
  repository other than the server's startup one stays reachable by naming it
  explicitly. The `gmem` CLI stays unrestricted.
- The **What** / **Why** / **Where** / **Learned** shape for durable saves,
  documented in the `graphmem-mcp-for-dev` skill and in the session guidance
  the Claude Code, OpenCode, and pi plugins inject.

### Changed

- Session and subagent guidance now tells agents to revise a superseded memory
  with `update` instead of storing a duplicate, and names the `memory_type`
  vocabulary (`decision`, `convention`, `constraint`, `incident`,
  `observation`) that the new recall filter matches on.
- An update that changes only `memory_type` or `importance` no longer loads the
  embedding model and no longer discards the stored vector. Neither field is
  part of the embedded document, so such a change used to fail when the model
  was unavailable, and drop a still-valid vector when embeddings were disabled.
- `Database::update_memory` is now a wrapper over `update_memory_with_vector`,
  mirroring how `remember_with_graph` wraps `remember_with_graph_and_vectors`.

## [0.1.1] - 2026-09-16

Versions up to 0.1.1 predate this changelog; see the commit history for their
contents.

[unreleased]: https://github.com/sonic182/graphmem/compare/0.9.2...HEAD
[0.9.2]: https://github.com/sonic182/graphmem/releases/tag/0.9.2
[0.9.1]: https://github.com/sonic182/graphmem/compare/0.9.0...0.9.1
[0.9.0]: https://github.com/sonic182/graphmem/compare/0.8.0...0.9.0
[0.8.0]: https://github.com/sonic182/graphmem/compare/0.7.1...0.8.0
[0.7.1]: https://github.com/sonic182/graphmem/compare/0.7.0...0.7.1
[0.7.0]: https://github.com/sonic182/graphmem/compare/0.6.0...0.7.0
[0.6.0]: https://github.com/sonic182/graphmem/compare/0.5.0...0.6.0
[0.5.0]: https://github.com/sonic182/graphmem/compare/0.4.0...0.5.0
[0.4.0]: https://github.com/sonic182/graphmem/compare/0.3.2...0.4.0
[0.3.2]: https://github.com/sonic182/graphmem/compare/0.3.1...0.3.2
[0.3.1]: https://github.com/sonic182/graphmem/compare/0.3.0...0.3.1
[0.3.0]: https://github.com/sonic182/graphmem/compare/0.2.0...0.3.0
[0.2.0]: https://github.com/sonic182/graphmem/compare/0.1.1...0.2.0
[0.1.1]: https://github.com/sonic182/graphmem/releases/tag/0.1.1
