---
name: graphmem-memory-sync
description: Saves what a session or work block taught into Graphmem (gmem) memory in one pass - decisions with their rationale, conventions, constraints, resolved failure causes and owner preferences - revising stale memories instead of duplicating them. Use only when the user asks to sync memory at the end of a session or block, for example "sync memory" or "save what we learned", or the equivalent in other languages. Do not use for routine recall or remember during a task; that belongs to graphmem-mcp-for-dev.
compatibility: Requires the Graphmem (gmem) MCP server to be connected
---

# Sync memory

An explicit end-of-session pass. The memory shape, the entity rules and the
tool rules are in `graphmem-mcp-for-dev`; load it first and do not restate it.

## Steps

1. List the durable facts of the session, from the conversation and the diff:
   decisions with their rationale, conventions, constraints, resolved failure
   causes, and the owner's stated preferences. Drop everything else.
2. For each fact, `recall` its topic first (`limit` 3 to 5). If recall is
   thin, `list` with a `memory_type` to see what is stored.
3. Pick one action per fact:
   - A memory covers it and is still right: do nothing.
   - A memory covers it but is stale or incomplete: `update` it with the full
     rewritten content, keeping the history that still matters.
   - Nothing covers it: `remember` it in the What/Why/Where/Learned shape,
     with entities and relations attached, and a fitting `memory_type`.
   - A memory is wrong or fully superseded: `inspect` it, then `forget` it.
     If it only changed, `update` it instead.
4. Keep writes in the current repo scope, or `global` for knowledge that is
   reusable across repositories.

## Never store

Secrets, credentials, private personal data, temporary progress, raw command
output, or code the repository already holds.

## Report

End with three short lists: created, updated, forgotten (id and one line
each). If nothing was durable, say so and write nothing.
