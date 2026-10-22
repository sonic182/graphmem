---
name: graphmem-memory-compact
description: Compacts a Graphmem (gmem) store by merging many memories on one topic into fewer dense ones, without losing facts that still hold or the graph links that make them recallable. Use only when the user asks to compact or pack memories, for example "compact memories", "pack memories" or "merge duplicate memories", or the equivalent in other languages. Do not use for routine recall or remember during a task; that belongs to graphmem-mcp-for-dev.
compatibility: Requires the Graphmem (gmem) MCP server with the list tool
---

# Compact memories

A maintenance pass. The memory shape and the tool rules are in
`graphmem-mcp-for-dev`; load it first and do not restate it.

## Steps

1. List the memories with `list` (`limit` 100): the current repo plus
   `global`, newest first, each with its content and entities. Repeat with
   `offset` set to `next_offset` until it is null. Pass `scopes` for another
   repo you may write to, and `inspect` for one record's relations. Without
   the `list` tool, use the CLI: `gmem list --limit 500`, `gmem show <id>`.
2. Group by topic. Within a group, find duplicates, overlaps and stale details.
3. Check doubtful facts against the current code or repo. Drop removed
   features and finished progress. Keep decisions that still explain the code.
4. Write each merged memory FIRST with `remember`: the union of the
   still-valid entities and relations, the highest importance of the group, and
   one `memory_type` (decision, convention, constraint, incident, observation).
5. THEN `forget` the old ids (`inspect` first if unsure). The order matters:
   `update` keeps only the target's entities, and `forget` drops the old
   links, so a new memory carrying the union keeps graph recall.
6. Keep each memory under the embedding limit: check it with
   `gmem tokens --id <id>` (limit in the output), or `gmem tokens --over-limit`
   at the end, when the CLI is available. Otherwise read `gmem://embedding`
   (about 450 tokens for a 512-token model). Split a topic rather than exceed
   it; a `warnings` field on `remember` or `update` means it was truncated.
7. Keep a log of old id -> new id while you work, and show it at the end.

## Scope

Write only in the current repo scope or `global`. Global memories are shared by
every repo: merge only clear duplicates there.

## Finish

Run `stats`, then a few `recall` checks on key topics to confirm the merged
memories are found. Report the id log and the before/after memory counts.
