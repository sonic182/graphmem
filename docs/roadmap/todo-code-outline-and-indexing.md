# TODO: code outlines and a local code index

## Goal

Let an agent find definitions and navigate source without confusing generated code
facts with Graphmem's durable, user-verified memories. Target languages:
Rust, Go, Zig, C, C++, Python, JavaScript/JSX, TypeScript/TSX, Elixir/Phoenix
(`.ex`, `.exs`, inline `~H`, `.heex`, `.eex`, including `.html.heex` and
`.html.eex`), Ruby, PHP, SQL, Bash, CSS, SCSS, and HTML (`<script>`,
`<style>`).

## Boundaries

- Keep code symbols in a separate, rebuildable SQLite index
  (`$GRAPHMEM_HOME/code-v{N}.sqlite`), not the existing `entities`/`edges` tables or
  memory `recall` ranking. No automatic memory extraction from source.
- Key an index by checkout root, not just Git's common directory: linked
  worktrees share memories but can contain different code. Only read files
  beneath the requested checkout; reject path escapes and do not follow
  symlinks out of it.
- Record definitions, imports, source ranges, and *syntactic* containment. Do
  not label name-matched call sites as resolved `CALLS` edges. No Cypher,
  watchers, semantic code embeddings, or background daemon.
- Two switches: the `code` Cargo feature compiles the tools in (off for a local
  `cargo build`, on in CI and release builds); `[code] enabled` (default
  `true`) or `GRAPHMEM_CODE=off` hides them at runtime without listing them.

## 1. Prove parser and template coverage first

- [x] Use the in-process crates `ast-grep-core`, `ast-grep-language` and
      `ast-grep-outline` 0.45.3. `ast-grep-language` has no per-language
      features, only one implicit feature per optional grammar
      (`tree-sitter-bash`, …); `default-features = false` plus those keeps the
      build to the grammars we use. Parsing a grammar that is not compiled in
      panics, so bundled outline rules are filtered before they are compiled.
- [x] Reuse bundled outline rules for JS/JSX, TS/TSX, Python, Ruby and PHP.
      They stop at item → member, so `module A; class B; def c` lost `c`;
      each item's children are outlined again and duplicates dropped.
- [x] Elixir: a syntax walk (not per-form patterns) finds
      `defmodule`/`defprotocol`/`defimpl`, `def`/`defp`/`defdelegate`,
      `defmacro(p)`, `defguard(p)` in plain, guarded and one-line forms, named
      `name/arity`, with consecutive clauses merged; `alias`/`import`/
      `require`/`use` are imports. Nested modules nest by range.
- [x] HEEx via `tree-sitter-heex` 0.8.1 (MIT, `tree-sitter-language` 0.1,
      compatible with ast-grep's tree-sitter 0.27) through a small
      `Language`/`LanguageExt` impl. Outline exposes `<.component>`,
      `<Module.component>` and `<:slot>`; plain tags are never symbols. `~H`
      sigils in `.ex` files are re-parsed as HEEx at their file position.
- [x] EEx: `tree-sitter-eex` is not usable (not on crates.io, tree-sitter
      0.20, crate misnamed `tree-sitter-heex`, no license file). Its grammar
      is only `<% %>` directives around text, so a scanner emits each
      directive as an `expression`; coverage is always
      `partial: EEx directives only`.
- [x] SQL via `tree-sitter-sequel` 0.3.11 (MIT): every `create_*` statement
      (table, view, index, function, trigger, type, …). Coverage is partial
      only when a `CREATE` keyword falls outside a parsed `create_*`
      statement: pg_dump's `\restrict` lines and unsupported column defaults
      are errors the grammar reports but they hide no symbol. Its `cc ~1.2.1`
      build-dependency pin downgrades the lockfile's `cc`; re-check on upgrades.
- [x] Bash `function_definition`; HTML `<script>` bodies outlined as
      JavaScript at their file position.
- [x] CSS via ast-grep's grammar and SCSS via `tree-sitter-scss` 1.0.0 (MIT),
      with one syntax walk: rule sets named by their selector list,
      `@media`/`@supports`, `@keyframes`, `@mixin`/`@function`, `--custom` and
      `$scss` variables, and `@import`/`@use`/`@forward` as imports. HTML
      `<style>` and HEEx `<script>`/`<style>` bodies (including `~H`) are
      outlined at their file position; HEEx has no raw-text node, so the body
      is sliced between the start and end tags. The SCSS grammar rejects
      `@extend %placeholder`, which makes coverage partial.
- [x] Rust, Go, C and C++ reuse ast-grep's grammars and bundled rules. Rust
      `impl` blocks are named `impl <Type>`, and C++ `namespace` blocks are
      added so namespaced functions get a parent. `.h` headers are parsed as
      C++, whose grammar also accepts nearly all C.
- [x] Zig via `tree-sitter-zig` 1.1.2 (MIT), with a syntax walk: functions,
      `const X = struct/enum/union/opaque/error{…}` containers, their
      fields, `@import`s, and `test "…"` blocks. Other constants are left out.

**Gate:** met on `tests/fixtures/code`. Files with syntax errors report
`partial: syntax errors`, never `complete`.

## 2. Index only what navigation needs

- [x] Discover files with `git ls-files --cached --others --exclude-standard`;
      skip symlinks, binary files, `*.min.js`, files over 1 MiB, and anything
      past 20,000 source files (reported as truncated).
- [x] Store checkout root, relative path, language, size + mtime stamp,
      coverage, and symbols (name, kind, parent, start/end, signature). A
      schema/extractor version in `PRAGMA user_version` rebuilds the index on
      mismatch. Bump `INDEX_VERSION` whenever extraction output changes.
- [x] Refresh is incremental and automatic: `find_symbol` refreshes the whole
      checkout (a no-op run stats files only: 0.18 s at 20,000 files) and
      `code_outline` refreshes its file. Changed files are parsed in parallel
      on a rayon pool that lives only for that run (`[code] index_threads`,
      `"auto"` = `available_parallelism`) and written 256 per transaction;
      deleted files are removed.
- [x] `[code] max_files` / `GRAPHMEM_CODE_MAX_FILES` replaces the fixed
      20,000-file bound.

## 3. Expose a small read-only interface

- [x] `code_outline(path, offset?, limit?, root?)`: paginated symbols with
      ranges and coverage, always fresh.
- [x] `find_symbol(query, kind?, limit?, root?)`: bounded matches with a
      `total`, exact names first, then last-segment and prefix matches, with
      paths and parents; never a single "resolved" target. Imports and
      template usages only when `kind` asks for them.
- [x] No MCP index tool: `find_symbol` and `code_outline` refresh on demand;
      `gmem code index` warms up a checkout from the CLI.
- [x] `gmem code index|outline|find`; progress on stderr.

## 4. Validate value before expanding

- [x] Integration tests: every language, edits/deletes, ignored files, path
      traversal and symlinks, stale results, two worktrees, config off; the
      memory tools' behavior unchanged (`tests/code.rs`, `tests/mcp.rs`).
- [ ] Compare common navigation questions against `rg`/`ast-grep` on real
      Phoenix and Python repositories. Ship more only if the index saves tool
      calls or improves precision; measure misses and false matches, not only
      timing.
- [ ] Content hash in the stamp if size + mtime ever reports a changed file as
      fresh.
- [x] Isolate binaries with different `INDEX_VERSION`s sharing one
      `$GRAPHMEM_HOME` (e.g. a plugin and a dev build) by naming the index
      `code-v{N}.sqlite`. Old caches, including legacy `code.sqlite`, are
      left untouched; each version rebuilds its own index from source.
- [ ] Code tools hold a `std::sync::Mutex` on a Tokio worker for a whole
      refresh (seconds on a first index); several parallel `find_symbol`
      calls can stall the memory tools. Run them through `spawn_blocking`.
- [ ] Checkout rows are never pruned: removed worktrees, temp clones, and any
      absolute `root` an agent passed keep their rows. Drop checkouts whose
      root no longer exists, e.g. during a refresh.
- [ ] Only then consider references, imports across files, or a call graph.
      Add an edge only when its target can be resolved reliably; carry
      uncertainty/coverage rather than presenting guesses as facts.

## References

- [ast-grep outline crate](https://github.com/ast-grep/ast-grep/tree/25334496c105c9c728f0a6024f3d164aed40cacc/crates/outline) — library API and bundled rules;
  [language crate](https://github.com/ast-grep/ast-grep/tree/25334496c105c9c728f0a6024f3d164aed40cacc/crates/language) — grammar features and HTML injection.
- [codebase-memory-mcp](https://github.com/DeusData/codebase-memory-mcp/tree/80eb92a7017dab9a0773430433660a966b80cc15) — examples of file outlines, coverage
  reporting and the cost of call resolution; inspiration, not a design to copy.
- [Phoenix Tree-sitter HEEx](https://github.com/phoenixframework/tree-sitter-heex)
  and [Tree-sitter EEx](https://github.com/connorlay/tree-sitter-eex).
