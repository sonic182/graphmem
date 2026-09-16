---
name: graphmem-mcp-for-dev
description: Recalls and stores durable project knowledge with the Graphmem (gmem) MCP server - past decisions and their rationale, repository conventions, invariants and constraints, resolved failure causes, and relationships between components, crates and services. Use at the start of a coding, debugging, refactoring, review, planning or maintenance task to recover context that is not in the code, and afterwards to record what the next session would otherwise have to work out again. Also use when the user asks what was decided before, why something is the way it is, or to remember or forget a project fact, and whenever the gmem tools recall, remember, relate, graph, inspect or forget are in play.
compatibility: Requires the Graphmem (gmem) MCP server to be connected
---

# Graphmem MCP for software development

Use this skill when Graphmem/gmem is connected and a coding, debugging, review, planning, or maintenance task could benefit from durable project context. Do not use it for ordinary questions that have no future project value.

Graphmem has two separate stores:

- Narrative memory is scoped, searchable text. Use `recall`, `remember`, `inspect`, and `forget`.
- The entity graph is unscoped structured relationships. Use `graph` and `relate`.

The stores stay separate, but `recall` ranks over both: it scores memories by meaning, scores the entities and relations the query matches, and propagates that rank along graph edges. A memory that shares no vocabulary with the query is therefore still reachable through the entities it is attached to.

That reachability is the reason to attach entities. A memory stored without them can only ever be found by its own wording, so the graph contributes nothing to finding it. Attaching entities on `remember` is the single highest-value habit in this skill.

## During development

Before investigating or changing code for every substantive task, call `recall` exactly once with `limit: 3` to `5` and a concise query containing the concrete component, symbol, error, or decision. Embeddings plus graph context are the default, so natural language and paraphrases are useful. Skip this only for direct no-code or logistical requests, or when Graphmem is unavailable.

Set `use_embeddings: false` only when you need an exact-token lookup, such as a literal symbol, error string, or file path; in that mode the query is FTS5, so use short keywords, quoted phrases, a trailing `*` for a prefix, and uppercase `OR` or `NOT`, because whitespace means AND.

Read the result critically. `recall` always returns its best candidates, even when the store holds nothing relevant, and scores are relative within one query rather than an absolute measure of relevance. Treat a result whose score is far below the top one, or whose content does not actually address the task, as "nothing known" and continue from the code.

Do not repeatedly recall the same context within a task. Use `stats` only to diagnose the local store, not as a routine step.

Before saving a fact on the same topic, recall it first. Prefer the existing memory when it is still correct rather than adding a duplicate.

## Store durable engineering context

Use `remember` after work only for verified information that is likely to help a future engineering task:

- architecture or API decisions and their rationale;
- repository conventions, commands, invariants, and constraints;
- significant resolved failure causes or compatibility requirements;
- stable preferences that affect future implementation work.

Make the content concise and self-contained: state the fact or decision, why it exists when that is non-obvious, and the affected area. When the memory establishes verified retrieval context, attach its entities and directed relations in the same `remember` call; this links the memory atomically. Use a descriptive `memory_type` such as `decision`, `convention`, `constraint`, or `incident`; reserve higher `importance` for information whose absence is likely to cause a wrong or costly change.

Do not store secrets, credentials, private personal data, unverified speculation, temporary progress updates, raw debugging output, or source code that can be read directly from the repository.

Omit `scopes` when the MCP server was started in the target repository. Use `repo:/absolute/path/to/repository` when the target differs from the server's startup repository. Use `global` only for knowledge that is genuinely reusable across repositories. Scoped recall includes matching global memories.

For example:

```json
{
  "content": "Decision: integration tests use cargo nextest because the project toolchain standardizes it; keep new integration coverage compatible with nextest.",
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

## Correct or remove memory safely

Use `inspect` to confirm a memory's content and ID before `forget`. `forget` permanently deletes a single narrative memory; do not use it to remove graph entities or relationships. When a prior decision has changed, save the replacement decision with its rationale rather than deleting history unless the older entry is erroneous or must be removed.
