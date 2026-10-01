---
name: graphmem-code-analysis
description: Locates definitions, declared imports, and file structure with the Graphmem (gmem) find_symbol, code_outline, and code_imports MCP tools, instead of grepping or reading whole files. Use when finding where a function, class, module, component, SQL object, or CSS selector is defined, listing imports declared by a file, outlining a file before reading or editing it, following a stack trace to its source, reviewing, refactoring, or renaming code, when a plan needs exact path:line citations, and whenever these tools are listed. Covers when to fall back to rg or ast-grep for call sites and references, which these tools do not index.
compatibility: Requires the Graphmem (gmem) MCP server with its code tools enabled (built with the code feature, and not turned off by `[code] enabled = false` or GRAPHMEM_CODE=off)
---

# Code analysis with Graphmem

`find_symbol` and `code_outline` read a syntax index of the Git checkout. Both
refresh it themselves before answering, re-reading only files whose size or
modification time changed, so there is no index step to run first.
`code_imports` lists the imports declared in one file. These tools expose
definitions and declared imports only: they do not resolve imports to files or
show call sites, references, types, or arbitrary text.

## Default workflow

1. You have a name → `find_symbol`. You have a file → `code_outline`.
2. Read only the returned `start_line`..`end_line` range, not the whole file.
3. For who calls or references it, use `rg -w <name>` or `ast-grep`. The gmem
   tools do not see usages, so never conclude "unused" from them.
4. When the tools are not listed, or the file's language is not supported (see
   the tool description), use `rg`/`ast-grep` from the start.

## By task

| Task | First step | Then |
| --- | --- | --- |
| Where is X defined? | `find_symbol` X | read that range |
| Which imports does a file declare? | MCP `code_imports` or `gmem code imports <FILE>` | inspect the import names and source ranges; this does not resolve them to files |
| Understand or edit a large file | `code_outline` path with `depth: 0` or `1` | raise `depth` only where needed, then read only the symbols involved |
| Stack trace or error names a function | `find_symbol` with `kind: "function"` or `"method"` | read the frame's range |
| Review or refactor a diff | `code_outline` each changed file | `rg`/`ast-grep` for callers of what changed |
| Rename, or impact of a signature change | `find_symbol` for the definition | `rg -w`/`ast-grep` for every reference |
| Plan that cites locations | `code_outline` the files involved | cite `path:start_line` from the result |
| Code in another checkout | either tool with `root` set to an absolute path inside it | same as above |

## Querying find_symbol

Matching ignores case and ranks in this order:

1. Exact name. An Elixir name also matches `name/arity`: `render` finds
   `render/2`. Querying `render/2` ranks that arity first but can also return
   prefix matches such as `render/20`; verify the returned name for an exact arity.
2. Name ending in `.query`: `ConsentLive` finds `MyAppWeb.ConsentLive`, `users`
   finds `public.users`, and `btn-primary` finds the CSS rule named
   `.btn, .btn-primary`.
3. Prefix: `fetch_` finds `fetch_user`.

Narrow a common name with `kind`. Raise `limit` (up to 100) when `total` is
greater than the matches returned. Read
[references/symbol-kinds.md](references/symbol-kinds.md) when you need a
`kind` value for a language, or when a language's names or kinds do not look
the way you expected.

## Gotchas

- Several matches are an ambiguity to resolve with `path` and `parent`, not a
  ranking: the first match is not the resolved target, and the tool never
  picks one.
- An empty result does not prove a name is absent. It may be past
  `[code] max_files` (`truncated: true`), in a file Git ignores, generated,
  minified (`.min.js`, `.min.css`), or in an unsupported language. Confirm with
  `rg` before saying it does not exist.
- Imports, HEEx `component`/`slot` usages, and EEx `expression`s are left out
  unless `kind` asks for them (`"import"`, `"component"`, `"slot"`,
  `"expression"`). Use `find_symbol` with `kind: "import"` to locate matching
  import declarations across the checkout, or `code_imports` to list every
  import declared by one file. Imports are syntax only: neither tool resolves
  an imported name to a file or dependency. Only import forms the parser
  recognises are listed, even when `coverage` is `complete`; loading through
  other APIs is not: `rg` for those when absence matters.
- `coverage: partial: <reason>` means symbols may be missing from the outline.
  EEx is always partial (directives only), and the SCSS grammar rejects
  `@extend %placeholder`. Fall back to reading or `rg` for the gap.
- `freshness: stale` or `missing` means the file changed between the refresh
  and the read. Call again rather than trusting the line numbers.
- `code_outline` returns at most 500 symbols per call. Continue from
  `next_offset` while it is present. With `depth`, `total` and `next_offset`
  count only the symbols within that depth; `index` and `parent` still refer
  to the full outline.
- `.h` headers are parsed as C++. HTML and HEEx `<script>`/`<style>` bodies are
  outlined at their real lines in the file, including inside Elixir `~H`.
- The first call in a large checkout that was never indexed can be slow. To
  build the index ahead of time, or to see which files were skipped, run
  `gmem code index` in the checkout.

## Relation to memory

Do not store symbol locations or outlines with `remember`: they are derivable
from the code and go stale. Decisions, conventions, and failure causes about
the code are durable; store those following the `graphmem-mcp-for-dev` skill.
