# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
- The **What** / **Why** / **Where** / **Learned** shape for durable saves,
  documented in the `graphmem-mcp-for-dev` skill and in the session guidance
  the Claude Code, OpenCode, and pi plugins inject.

### Changed

- Session and subagent guidance now tells agents to revise a superseded memory
  with `update` instead of storing a duplicate, and names the `memory_type`
  vocabulary (`decision`, `convention`, `constraint`, `incident`,
  `observation`) that the new recall filter matches on.
- `Database::update_memory` is now a wrapper over `update_memory_with_vector`,
  mirroring how `remember_with_graph` wraps `remember_with_graph_and_vectors`.

## [0.1.1] - 2026-09-16

Versions up to 0.1.1 predate this changelog; see the commit history for their
contents.

[unreleased]: https://github.com/sonic182/graphmem/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/sonic182/graphmem/releases/tag/v0.2.0
[0.1.1]: https://github.com/sonic182/graphmem/commits/master
