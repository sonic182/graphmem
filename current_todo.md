# Graphmem — Code Index Improvements TODO

## Top 3 priorities

| Rank | Improvement | Effort | Value |
|---|---|---|---|
| 1 | Add depth filtering to `code_outline` so agents can start with a compact file overview and expand only where needed. | Low | High |
| 2 | Move synchronous memory MCP work to `spawn_blocking` so slow embedding or lock waits do not stall other MCP requests. | Medium | High |
| 3 | Add structural `code_diff` to show which symbols changed between Git revisions, reusing the existing outline parser. | Medium | High |

## Objective

Improve code navigation without turning Graphmem into an LSP, attempting
semantic reference resolution, or mixing the rebuildable code index with
durable memory.

## Principles

- Keep the code index local, rebuildable, and separate from the memory store.
- Do not add LSP or external language-server dependencies.
- Do not claim semantic resolution that we cannot demonstrate.
- Prefer structural information from parsers.
- Keep rg and ast-grep as fallbacks for usages and references.
- Avoid significant increases in memory use, complexity, or startup time.
- Preserve compatibility with supported languages.

---

## [x] P0 — Keep Tokio workers free during code-index refresh

**Completed in PR #27.**

Code-index operations that refresh the index, run Git, walk and stat files,
parse source, write SQLite, or wait for the index lock now run on Tokio's
blocking pool. Callers wait asynchronously for the index lock before entering
the blocking pool, and the owned guard moves into the blocking closure.

- [x] Identify the MCP code-index operations that perform blocking work.
- [x] Run that work with tokio::task::spawn_blocking.
- [x] Keep lock waits off both ordinary Tokio workers and queued blocking-pool
  threads.
- [x] Do not hold a lock across an async suspension.
- [x] Convert worker errors and panics into clean MCP errors.
- [x] Preserve behavior: find_symbol refreshes the checkout and code_outline
  refreshes the requested file before returning.
- [x] Keep memory tools usable while a code-index refresh is running.
- [x] Test concurrent code calls and a slow first index with one and two Tokio
  workers.
- [x] Verify an independent memory tool can answer before the slow code call.

**Follow-up coverage**

- [ ] Add a specific concurrency test for find_symbol running alongside a slow
  recall; the existing refresh test uses stats as the independent memory tool.

The initial goal was to prevent a heavy index operation from occupying an
ordinary Tokio worker for the duration of the refresh. It does not require
multiple concurrent SQLite writers.

---

## [x] P1 — Expose declared imports without semantic resolution

**Completed in PR #28.**

Graphmem reports import declarations written in source; it does not resolve
them to files or create file-dependency edges.

### CLI and MCP

- [x] Add gmem code imports <FILE>.
- [x] Add the code_imports MCP tool with root, offset, and limit.
- [x] Return total count, next offset, source line ranges, and parser coverage.
- [x] Refresh the file before returning imports.
- [x] Share the import filter between CLI and MCP through FileOutline::imports().
- [x] Preserve written import names, including language-specific quotes, and
  cap names at 160 characters.
- [ ] Optional future idea: add gmem code find-import <QUERY> to search import
  declarations across files.

### Language coverage

- [x] Rust use and extern crate.
- [x] Go imports.
- [x] Python import and from ... import.
- [x] JavaScript and TypeScript imports, require(), dynamic import(),
  TypeScript import = require(), and export ... from.
- [x] Elixir alias, import, require, and use.
- [x] SCSS @import, @use, and @forward.
- [x] C and C++ includes.
- [x] Zig @import.
- [x] Ruby require, require_relative, and load.
- [x] PHP require, require_once, include, and include_once.
- [x] Bash source and dot imports.

### Tests and acceptance

- [x] Cover imports across supported languages, including multiple imports.
- [x] Cover multiline source ranges and import forms.
- [x] Cover partial parser coverage.
- [x] Cover files changed since their previous index refresh.
- [x] Cover MCP pagination.
- [x] Keep imports as declared strings; do not claim that they resolve to files.

The original acceptance goal is met: an agent can ask what a file imports
without reading the whole file, and Graphmem makes no claim about the resolved
target.

---

## [x] P1 — Structural code diff

**Completed on branch feat/outline-depth-and-code-diff.** `gmem code diff
<BASE> [HEAD]` and the code_diff MCP tool compare the merge base with head,
outline both versions in memory, and never touch code.sqlite. A symbol is
modified when its signature or own text changed: its span plus the comment and
attribute lines directly above it, minus nested symbols, whitespace, `,`, and
`;`, so edits are not reported on enclosing impls or modules. The CLI has no
--json yet (the MCP tool returns JSON), head cannot be the working tree, and
the MCP response is not paginated.

Add a structural view of changes between two Git revisions. Git diff shows
changed lines; Graphmem should show which symbols changed without explaining
the change semantically.

### Proposed API

CLI:

~~~text
gmem code diff <BASE> <HEAD>
~~~

Example:

~~~text
src/application.rs
  ~ function update_memory
  + function reembed_memory
  - function legacy_update

src/domain.rs
  ~ struct Memory
~~~

Optional JSON output:

~~~text
gmem code diff HEAD~1 HEAD --json
~~~

MCP tool: code_diff, with base, head, and optional root.

### Initial approach

1. Get changed paths with git diff --name-status <base>...<head>.
2. Read each file version with git show <revision>:<path>.
3. Run the existing outline extractor on each version without persisting these
   temporary outlines in code.sqlite.
4. Compare symbols with a stable key such as kind, name, and parent.
5. Classify symbols as added, removed, modified, or unchanged. Initially,
   modified can mean the same identity exists on both sides but its signature
   or relevant range changed.

Do not attempt sophisticated rename detection initially. A rename may appear as
a removed old name and an added new name; that is acceptable.

### Files and tests

- [x] Handle added, deleted, modified, and Git-renamed files (renames are
  detected with git -M and compared old path against new path).
- [x] Ignore binary files and unsupported languages (listed as skipped).
- [x] Test added and removed functions.
- [x] Test signature and body changes.
- [x] Test changed classes and modules.
- [x] Test added and deleted files.
- [x] Test nested symbols.
- [x] Test Elixir multiple clauses.
- [x] Test HEEx and other templates.
- [x] Test CSS (SCSS shares the same style outliner).

Do not implement call-graph impact, affected callers, semantic explanations,
memory invalidation, LLM summaries, or sophisticated rename detection yet.

**Acceptance:** For a typical PR, one code_diff call returns the structurally
affected symbols so an agent can decide which source ranges to inspect.

---

## [ ] P2 — Add lightweight metadata to symbols

Add useful structural metadata to CodeSymbol without turning the index into a
semantic database.

### Visibility

Where it can be derived reliably, record visibility with a stable value such
as Public, Private, Protected, Internal, or Unknown.

- Rust: pub, pub(crate), and private.
- Elixir: def is public; defp is private.
- Python: usually unknown. Do not infer privacy from a leading underscore
  unless that heuristic is explicitly documented.
- C++: public, private, and protected when reliably available from the AST.

### Documentation

Optionally record an immediately associated comment or docstring:

~~~rust
pub documentation: Option<String>
~~~

Use a reasonable limit, such as 512–1,024 bytes. Do not store symbol bodies.
Potential sources include Rust doc comments, Python docstrings, JavaScript or
TypeScript JSDoc, and Elixir @doc when it can be extracted reliably.
Leave the field empty when extraction would require fragile heuristics.

### Schema, API, and tests

- [ ] Add visibility and documentation fields to the symbol schema.
- [ ] Bump INDEX_VERSION because extracted output changes.
- [ ] Return available metadata from find_symbol and code_outline.
- [ ] Consider a future visibility filter; it is not required initially.
- [ ] Test Rust and Elixir visibility.
- [ ] Test C++ access modifiers if implemented.
- [ ] Test documentation comments and docstrings.
- [ ] Test missing metadata and the documentation size limit.

**Acceptance:** An agent can distinguish public APIs from private helpers
without reading the entire symbol body.

---

## [x] P1 — Add depth filtering to code_outline

**Completed on branch feat/outline-depth-and-code-diff.** Filtering lives in
FileOutline::within_depth(), shared by CLI and MCP; index and parent keep their
full-outline values. Depth is language-agnostic (it only follows parent), so
tests cover Elixir nesting and MCP pagination rather than every language.

Let callers limit how many levels of nested symbols code_outline returns.
The default must preserve current behavior.

### Proposed API and semantics

MCP:

~~~json
{
  "path": "src/application.rs",
  "depth": 1
}
~~~

CLI:

~~~text
gmem code outline src/application.rs --depth 1
~~~

- depth = 0: top-level symbols only.
- depth = 1: top-level symbols and their direct children.
- depth = 2: include one more nesting level.
- Unspecified depth: current unlimited behavior, for compatibility.
- A null depth may represent unlimited output if useful.

Filter by depth before applying pagination so total, offset, and next_offset
remain consistent. Use existing parent_ordinal data rather than changing the
parser or duplicating hierarchy information. A parent filter such as MyApp.User
is a possible later improvement, not required now.

Useful cases include large Elixir modules, C++ namespace/class hierarchies, and
nested HTML or HEEx components. Update the code-analysis skill to recommend
starting with a shallow outline for large or unfamiliar files and increasing
depth only when needed.

### Tests

- [x] Test depths 0 and 1 (depth 2 is the unlimited output of the fixture).
- [x] Test default and unlimited behavior.
- [x] Test symbols without parents and multiple nesting levels.
- [x] Test pagination after depth filtering.
- [x] Test Elixir nested modules.
- [ ] Test C++ namespace, class, and method nesting (skipped: language-agnostic).
- [ ] Test Python class, method, and nested-function structure (skipped: language-agnostic).
- [ ] Test HTML and HEEx nesting (skipped: language-agnostic).

**Acceptance:** An agent can inspect a file's high-level structure without
receiving every nested symbol.

---

## [ ] P2 — Follow up on blocking work

Two items were intentionally left outside the code-index spawn_blocking
change.

### Synchronous memory tools

The memory tools in src/mcp.rs still perform synchronous work while holding a
std::sync::Mutex. In particular, recall can load and run the embedding model on
a Tokio worker, and callers waiting for the lock also occupy workers.

- [ ] Move memory-tool work to spawn_blocking with a helper similar to
  MemoryServer::run_code.
- [ ] Add a concurrency test with GRAPHMEM_TOKIO_WORKER_THREADS=1: a slow
  recall should not block another tool.

### Poisoned code-index mutex

A panic while holding the code-index lock can poison it. Then find_symbol and
code_outline return “code index lock is poisoned” until the server restarts.
Memory tools use a separate lock and are unaffected.

- [ ] Decide whether to recover with PoisonError::into_inner (the state is
  stored in SQLite and dropped transactions roll back) or keep returning a
  clear error.

---

## Validation

The merged import change reported just verify passing (79 tests, 3 skipped)
and git diff --check clean.

For future Rust changes, follow the repository feedback loop:

- [ ] Run package-scoped just check, just test, and just lint while iterating.
- [ ] Run just fmt after Rust edits.
- [ ] Run just verify before handoff.
- [ ] Test builds with and without the code feature when affected.
- [ ] Validate the Windows MSVC build when affected.
- [ ] Smoke-test changed CLI and MCP behavior.
- [ ] Keep MCP protocol output on stdout and application logs on stderr.

Also verify the project invariants:

- [ ] Memory tools remain usable if code indexing fails.
- [ ] No LSP dependency is introduced.
- [ ] Code symbols remain outside the memory graph.
- [ ] Imports and usages are not presented as semantically resolved references.
- [ ] The code index remains fully rebuildable.

---

## Benchmark before adding broader navigation features

Before adding features such as find_references or a call graph, measure the
workflow on at least one Python repository and one Phoenix/Elixir repository.

Record tool calls, source lines read, approximate tokens sent to the agent,
latency, and success or failure. Compare an rg-first workflow with a
Graphmem-first workflow for:

- [ ] Finding a definition.
- [ ] Understanding a large file's structure.
- [ ] Identifying structural changes in a PR.
- [ ] Finding a file's imports.
- [ ] Finding a public API.

Do not implement find_references, a call graph, or LSP support unless
measurements show a concrete need.

**Suggested next priorities:** code_outline depth and structural code_diff.
The depth option may offer a strong value-to-effort ratio because symbol
parents are already stored as parent_ordinal. Code-index spawn_blocking,
declared-import listing, code_outline depth, and structural code_diff are
complete.
