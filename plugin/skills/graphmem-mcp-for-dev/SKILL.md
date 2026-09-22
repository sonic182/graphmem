---
name: graphmem-mcp-for-dev
description: Recalls and stores durable project knowledge with the Graphmem (gmem) MCP server - past decisions and their rationale, repository conventions, invariants and constraints, resolved failure causes, and relationships between components, crates and services. Use at the start of a coding, debugging, refactoring, review, planning or maintenance task to recover context that is not in the code, and afterwards to record what the next session would otherwise have to work out again. Also use when the user asks what was decided before, why something is the way it is, or to remember, revise, or forget a project fact, and whenever the gmem tools recall, remember, update, relate, graph, inspect or forget are in play.
compatibility: Requires the Graphmem (gmem) MCP server to be connected
---

# Graphmem MCP for software development

Use this skill when Graphmem/gmem is connected and a coding, debugging, review, planning, or maintenance task could benefit from durable project context. Do not use it for ordinary questions that have no future project value.

Graphmem has two separate stores:

- Narrative memory is scoped, searchable text. Use `recall`, `remember`, `update`, `inspect`, and `forget`.
- The entity graph is unscoped structured relationships. Use `graph` and `relate`.

The stores stay separate, but `recall` ranks over both: it scores memories by meaning, scores the entities and relations the query matches, and propagates that rank along graph edges. A memory that shares no vocabulary with the query is therefore still reachable through the entities it is attached to.

That reachability is the reason to attach entities. A memory stored without them can only ever be found by its own wording, so the graph contributes nothing to finding it. Attaching entities on `remember` is the single highest-value habit in this skill.

## During development

Before investigating or changing code for every substantive task, call `recall` exactly once with `limit: 3` to `5` and a concise query containing the concrete component, symbol, error, or decision. Embeddings plus graph context are the default, so natural language and paraphrases are useful. Skip this only for direct no-code or logistical requests, or when Graphmem is unavailable.

Set `use_embeddings: false` only when you need an exact-token lookup, such as a literal symbol, error string, or file path; in that mode the query is lexical: words, quoted phrases, and `prefix*` terms match if any of them does, and BM25 ranks memories that share more of them first; an uppercase `AND`, `OR`, `NOT`, or `NEAR` switches to exact FTS5 syntax (use `AND` to require every word).

A `remember`, `recall`, or `relate` response can include a `warnings` field when an input was longer than the embedding model's token limit and was truncated before embedding. That content did not affect ranking; use `recall` with `use_embeddings: false` to search the full text lexically, or split the memory so each part fits. Read the `gmem://embedding` resource for the active model, revision, and its exact `max_tokens` limit.

Read the result critically. `recall` always returns its best candidates, even when the store holds nothing relevant, and scores are relative within one query rather than an absolute measure of relevance. Treat a result whose score is far below the top one, or whose content does not actually address the task, as "nothing known" and continue from the code.

Do not repeatedly recall the same context within a task. Use `stats` only to diagnose the local store, not as a routine step.

Before saving a fact on the same topic, recall it first. Prefer the existing memory when it is still correct, and `update` it when it is not, rather than adding a duplicate.

## Store durable engineering context

Use `remember` after work only for verified information that is likely to help a future engineering task:

- architecture or API decisions and their rationale;
- repository conventions, commands, invariants, and constraints;
- significant resolved failure causes or compatibility requirements;
- stable preferences that affect future implementation work.

Make the content self-contained, and give a durable save this shape:

```markdown
**What**: the fact or decision, in one sentence.
**Why**: what motivated it - the bug, the constraint, the user's request.
**Where**: the files, paths, or components affected.
**Learned**: the gotcha or edge case worth knowing next time.
```

Omit `**Why**` when it is self-evident and `**Learned**` when there is none; a single fact such as a command or a version requirement may stay one line. The content is what gets embedded, so naming the affected files and the rationale inside it is also what makes the memory findable later by a question about those files.

When the memory establishes verified retrieval context, attach its entities and directed relations in the same `remember` call; this links the memory atomically. Use a `memory_type` from `decision`, `convention`, `constraint`, `incident`, or `observation` - `recall` can filter on it, so an ad-hoc category makes the memory harder to narrow to. Reserve higher `importance` for information whose absence is likely to cause a wrong or costly change.

Do not store secrets, credentials, private personal data, unverified speculation, temporary progress updates, raw debugging output, or source code that can be read directly from the repository.

Omit `scopes` when the MCP server was started in the target repository. Use `repo:/absolute/path/to/repository` when the target differs from the server's startup repository. Use `global` only for knowledge that is genuinely reusable across repositories. Scoped recall includes matching global memories.

For example:

```json
{
  "content": "**What**: integration tests run under cargo nextest.\n**Why**: the project toolchain standardizes it, and the justfile test recipe calls it directly.\n**Where**: justfile, tests/.\n**Learned**: nextest runs each test in its own process, so tests must not share global state.",
  "memory_type": "convention",
  "importance": 0.7,
  "entities": [
    { "kind": "crate", "name": "graphmem" },
    { "kind": "tool", "name": "cargo nextest" }
  ],
  "relations": [
    {
      "source": { "kind": "crate", "name": "graphmem" },
      "relation": "tested_with",
      "target": { "kind": "tool", "name": "cargo nextest" }
    }
  ]
}
```

## Store verified relationships

Use `graph` to inspect an entity before adding an uncertain or potentially duplicate relationship, and to inspect bounded context around a recall result. Use `relate` only for verified, durable graph-only facts such as a crate depending on a library, a service owning a component, or a module implementing an interface. If the fact belongs in a narrative memory too, use `remember` with entities and relations instead.

Choose stable entity kinds and names, and use a concise `snake_case` relation such as `depends_on`, `uses`, `owns`, or `implements`. The graph is unscoped, so do not add repository-specific or speculative relationships. `relate` creates missing entities and reuses an existing identical edge.

For example:

```json
{
  "source": { "kind": "crate", "name": "graphmem" },
  "relation": "uses",
  "target": { "kind": "library", "name": "rusqlite" }
}
```

## Revise instead of duplicating

When `recall` surfaces a memory that is now wrong, incomplete, or superseded, call `update` with its `id` rather than storing a second memory on the same topic. Competing memories on one topic dilute recall: both are returned, and neither says which one holds. `update` replaces only the fields you pass, keeps the rest, and leaves scopes, entities, and relations untouched; changed content is re-embedded in the same transaction.

Rewrite the full content when you update - it replaces the stored text, so a partial statement loses the rest of the record. Keep the history that still matters inside the new content, for example by stating what the decision replaced and why it changed.

Narrow a recall with `memory_type` when you want only one category, such as `decision` for what was decided about an area, or `constraint` for what must not change. Matching ignores case, and the filter is applied before ranking, so a filtered recall never returns another category. Use it to check an area's prior decisions before proposing a new one.

## Correct or remove memory safely

`inspect`, `update`, and `forget` take the same `scopes` argument as `remember` and `recall`, and default to the server's startup repository plus global. When you recalled a memory from another repository by passing `repo:/absolute/path/to/repository`, pass that same scope to inspect, revise, or remove it; without it the id is treated as out of scope and reported as not found, so a stale or guessed id cannot reach another repository's memory.

Use `inspect` to confirm a memory's content and ID before `update` or `forget`. `forget` permanently deletes a single narrative memory; do not use it to remove graph entities or relationships. Reserve `forget` for entries that were erroneous or must be removed; for a decision that simply changed, `update` it.
