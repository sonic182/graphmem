# TODO: code outlines and a local code index

## Goal

Let an agent find definitions and navigate source without confusing generated code
facts with Graphmem's durable, user-verified memories. First use must work on
Elixir/Phoenix (`.ex`, `.exs`, `.heex`, `.eex`, including `.html.heex` and
`.html.eex`), HTML, JavaScript/JSX, TypeScript/TSX, and Python. Do not start
with a Rust-only prototype.

## Boundaries

- Keep code symbols in a separate, rebuildable SQLite index, not the existing
  `entities`/`edges` tables or memory `recall` ranking. No automatic memory
  extraction from source.
- Key an index by checkout root, not just Git's common directory: linked
  worktrees share memories but can contain different code. Only read files
  beneath the requested checkout; reject path escapes and avoid following
  symlinks out of it.
- Initially record definitions, imports, source ranges, and *syntactic*
  containment. Do not label name-matched call sites as resolved `CALLS` edges.
  No Cypher, watchers, semantic code embeddings, or background daemon.

## 1. Prove parser and template coverage first

- [ ] Spike the in-process Rust libraries `ast-grep-outline` and
      `ast-grep-language` (currently 0.45.3); enable only the Elixir, HTML,
      JavaScript, TypeScript and Python grammar features. Add `ast-grep-core`
      directly only if custom structural matching needs it. Verify build,
      binary impact and extraction on representative files before committing
      the dependency choice.
- [ ] Reuse bundled outline rules for JS/JSX, TS/TSX and Python. Supply Elixir
      rules for modules, functions (`def`/`defp`, guards and one-line forms),
      macros and relevant Phoenix components. Check duplicates and nesting on
      actual Phoenix modules; the Elixir grammar alone does not supply an
      outline.
- [ ] Test Phoenix's `tree-sitter-heex` for HEEx and the separate
      `tree-sitter-eex` for EEx against ast-grep's Tree-sitter version. Neither
      is an ast-grep built-in: verify Rust crate availability, static grammar
      registration through its public language traits, licensing, and source
      ranges. If either is incompatible, choose and document a bounded
      template-specific fallback rather than treating templates as plain HTML.
- [ ] Define what a template outline should expose: component invocations,
      references to named components/modules when explicit, and embedded
      Elixir expressions with accurate file/line ranges. Test `<.component>`,
      `<Module.component>`, HEEx `{...}`, EEx `<%= ... %>`, and ordinary markup.
      Avoid indexing every HTML tag as a symbol. Check inline `~H` sigils in
      `.ex` files too; add injection handling if the target repos use them.
- [ ] For ordinary HTML, reuse ast-grep's `<script>` JS/TS injection support
      explicitly in our library integration; add HTML outline rules only for
      structures that prove useful. Do not assume the HTML parser understands
      HEEx/EEx. Check file-extension routing, including compound extensions.

**Gate:** each target file type produces useful symbols with correct paths and
ranges on a small, real-world fixture. Unsupported constructs must be visible
as coverage gaps, not silently reported as a complete index.

## 2. Index only what navigation needs

- [ ] Discover repository files with Git/ignore rules, bounded by file count
      and size; skip binary, generated, and untrusted paths. Use `ignore` only
      if existing Git-based discovery proves insufficient.
- [ ] Store checkout root, relative path, language, a content fingerprint,
      indexer/rule version, and symbol records (name, kind, parent, start/end
      positions, signature). Keep enough source identity to distinguish
      same-named functions in different modules/files; never infer a unique
      target from a short name alone.
- [ ] Make refresh explicit or on-demand initially. Re-index changed files and
      remove deleted ones; atomically replace each file's rows. On query,
      verify freshness before returning a source location and say `stale` or
      `not indexed` when it cannot be trusted. A failed parse must not publish
      a partial index as complete.

## 3. Expose a small read-only interface

- [ ] `code_outline(path)` returns paginated symbols for a repository-relative
      file with source ranges and a freshness/coverage status.
- [ ] `find_symbol(query)` returns bounded, disambiguated matches with paths,
      language and line numbers. Consider a snippet tool only if the existing
      file-read tools do not cover it.
- [ ] Provide an explicit CLI index/refresh operation before considering an
      MCP indexing tool. Keep MCP stdout strictly JSON-RPC; report indexing
      progress on stderr only.

## 4. Validate value before expanding

- [ ] Integration-test Python, JS/JSX, TS/TSX, Elixir and Phoenix templates;
      edits/deletes, two worktrees, ignored files, path traversal, stale
      results, empty/invalid files, and bounded outputs. Keep the memory store
      and `recall` behavior unchanged.
- [ ] Compare common navigation questions against `rg`/`ast-grep` with the
      same repositories. Ship if the index saves tool calls or improves
      precision; measure misses and false matches rather than only timing.
- [ ] Only then consider references, imports across files, or a call graph.
      Add an edge only when its target can be resolved reliably; carry
      uncertainty/coverage rather than presenting guesses as facts.

## References

- [ast-grep outline crate](https://github.com/ast-grep/ast-grep/tree/25334496c105c9c728f0a6024f3d164aed40cacc/crates/outline) — library API and bundled rules;
  [language crate](https://github.com/ast-grep/ast-grep/tree/25334496c105c9c728f0a6024f3d164aed40cacc/crates/language) — grammar features and HTML injection.
- [codebase-memory-mcp](https://github.com/DeusData/codebase-memory-mcp/tree/80eb92a7017dab9a0773430433660a966b80cc15) — examples of file outlines, coverage
  reporting and the cost of call resolution; inspiration, not a design to copy.
- [Phoenix Tree-sitter HEEx](https://github.com/phoenixframework/tree-sitter-heex)
  and [Tree-sitter EEx](https://github.com/connorlay/tree-sitter-eex) — candidate
  template grammars; compatibility has not yet been verified.
