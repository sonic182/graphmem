# graphmem — Local Graph Memory MCP for Coding Agents

## Status

**Stage:** Design / MVP  
**Primary language:** Rust  
**Distribution:** Single binary  
**Storage:** Embedded SQLite  
**Transport:** MCP over stdio  
**Primary clients:** Claude Code, Codex CLI  
**Data location:** `~/.memory/` by default

---

# 1. Problem

Coding agents such as Claude Code and Codex CLI are very capable inside a single session, but their long-term memory is fragmented.

Useful context is repeatedly lost between sessions:

- architectural decisions
- project conventions
- previous debugging discoveries
- known pitfalls
- user preferences
- relationships between repositories, components, libraries, problems and solutions
- reasons behind previous technical decisions

Existing memory solutions often introduce too much infrastructure:

- external graph databases
- PostgreSQL
- Neo4j
- Docker containers
- HTTP services
- hosted APIs
- Python environments
- Node.js packages
- multiple persistent processes

This is undesirable for a developer tool that should feel closer to `git`, `rg`, or `sqlite3`.

The desired experience is:

```text
install one binary
        ↓
configure it as an MCP server
        ↓
use it from Claude Code / Codex
        ↓
memory persists automatically
```

The system should require no daemon and no external database.

---

# 2. Goal

Build a small, local-first memory server distributed as a **single Rust binary**.

The binary should provide:

1. an MCP server over stdio
2. persistent local memory
3. scoped memories
4. graph relationships
5. full-text retrieval
6. a human-readable/debuggable CLI
7. optional semantic/vector search later

All persistent information should live under:

```text
~/.memory/
```

or the appropriate platform-specific data directory.

Example:

```text
~/.memory/
├── memory.sqlite
└── config.toml
```

No external service should be required.

---

# 3. Non-goals

The first version should NOT attempt to become:

- a general-purpose graph database
- a Neo4j replacement
- a vector database
- a distributed memory service
- a hosted SaaS
- a multi-user server
- an autonomous knowledge extraction platform
- a replacement for Git
- a replacement for project documentation

The priority is:

> A tiny, reliable memory layer for local coding agents.

---

# 4. Proposed architecture

```text
                ┌───────────────┐
                │  Claude Code  │
                └───────┬───────┘
                        │
                        │ MCP stdio
                        │
                ┌───────▼────────┐
                │                │
Codex CLI ─────▶│ memory binary  │◀──── Human CLI
                │                │
                └───────┬────────┘
                        │
                  embedded access
                        │
                ┌───────▼────────┐
                │     SQLite     │
                │                │
                │ memories       │
                │ scopes         │
                │ entities       │
                │ edges          │
                │ sources        │
                │ FTS5           │
                └────────────────┘

                ~/.memory/
```

The process starts when Claude Code or Codex launches the MCP server and exits when the client closes it.

No daemon is required.

---

# 5. Why SQLite

SQLite should be the default storage engine.

Use:

```text
rusqlite
+
bundled SQLite
```

The SQLite runtime should be compiled into the binary.

This gives us:

- one distributable binary
- transactions
- schema migrations
- indexes
- joins
- recursive graph queries
- FTS5 full-text search
- simple backup
- easy debugging
- mature tooling
- no database server

A graph database is not required for the expected dataset size.

Graph relationships can be represented using ordinary relational tables.

Example:

```text
nodes
edges
```

Recursive graph traversal can be implemented using SQLite recursive CTEs.

---

# 6. Why not RocksDB / LevelDB

RocksDB and LevelDB are optimized primarily around key/value access.

For this project they would require implementing and maintaining secondary indexes manually.

For example:

```text
memory:{id}

scope:{scope}:memories

entity:{entity}:memories

edge:{source}:{relation}:{target}

tag:{tag}:memories
```

This would recreate functionality SQLite already provides.

The expected workload is dominated by:

- filtering
- text search
- relationship lookup
- scoped retrieval
- ranking

rather than extremely high write throughput.

Therefore the default decision is:

> Use SQLite unless benchmarks demonstrate a real reason not to.

---

# 7. Core data model

Keep the schema deliberately small.

## Memory

A durable piece of remembered information.

Examples:

```text
"We use cargo nextest for integration tests."

"The repository pattern was rejected because SQLx queries live in services."

"Retrying this endpoint at the HTTP layer caused duplicate jobs."
```

Suggested fields:

```text
id
content
memory_type
importance
created_at
updated_at
last_accessed_at
access_count
```

Possible `memory_type` values:

```text
fact
decision
preference
convention
problem
solution
warning
observation
```

Do not enforce a complex ontology initially.

---

# 8. Scopes

Memories should not belong to a hardcoded user/project hierarchy.

Use generic scopes.

Examples:

```text
global

user:alvaro

repo:/Users/alvaro/src/project

project:search-api

session:019abc...
```

A memory may belong to multiple scopes.

Example:

```text
Memory:
"We use cargo nextest."

Scopes:
user:alvaro
repo:/Users/alvaro/code/foo
```

Suggested precedence:

```text
repository
    ↓
project
    ↓
user
    ↓
global
```

More specific scopes should rank higher during retrieval.

---

# 9. Entities

Entities represent things referenced by memories.

Examples:

```text
repository
project
component
library
language
person
service
database
concept
tool
```

Suggested structure:

```text
Entity

id
kind
name
canonical_name
created_at
updated_at
```

Do not create a large fixed ontology in v1.

Entity kinds should initially be strings.

---

# 10. Graph relationships

Graph edges connect entities, memories, or potentially both.

Conceptual examples:

```text
Decision
    └── affects ───────▶ Component

Problem
    └── solved_by ─────▶ Solution

Memory
    └── mentions ──────▶ Entity

Memory
    └── supersedes ────▶ Memory

Component
    └── depends_on ────▶ Component

Incident
    └── caused_by ─────▶ Problem
```

A minimal edge model:

```text
id
source_id
relation
target_id
created_at
metadata
```

Indexes should exist for:

```text
source_id
target_id
relation
```

Avoid exposing raw graph operations to the MCP client unless necessary.

The memory server should own graph semantics.

---

# 11. Search

Retrieval should initially avoid embeddings.

The MVP should combine:

```text
SQLite FTS5
+
scope specificity
+
graph proximity
+
importance
+
recency
```

Conceptual ranking:

```text
score =
    text_relevance
  + scope_relevance
  + graph_relevance
  + importance
  + recency
```

Exact weighting should remain configurable and should be benchmarked later.

Do not prematurely optimize ranking constants.

---

# 12. Vector search

Vector search is explicitly **not required for MVP**.

Possible later implementation:

```text
sqlite-vec
```

This preserves the single-database architecture:

```text
memory.sqlite
├── structured data
├── graph
├── FTS
└── vectors
```

Potential local embedding implementation:

```text
fastembed
```

Embeddings should only be added after evaluating retrieval quality using FTS + graph signals.

---

# 13. MCP interface

The MCP surface should remain intentionally small.

Initial tools:

```text
remember
recall
forget
inspect
```

Potential later tool:

```text
relate
```

Avoid exposing low-level operations such as:

```text
create_node
create_edge
delete_edge
query_sql
query_graph
update_entity
```

The agent should interact with **memory semantics**, not database semantics.

---

# 14. MCP tool: remember

Purpose:

Store useful durable information.

Example conceptual request:

```json
{
  "content": "This repository uses cargo nextest for integration tests.",
  "type": "convention",
  "scopes": [
    "repo:/Users/alvaro/src/foo"
  ]
}
```

Responsibilities:

```text
validate input
↓
normalize scopes
↓
store memory
↓
index in FTS
↓
optionally associate entities
↓
optionally create graph relations
```

Return:

```text
memory id
normalized scopes
created timestamp
```

---

# 15. MCP tool: recall

Purpose:

Retrieve the most relevant previous memories.

Conceptual request:

```json
{
  "query": "How do we run integration tests?",
  "scopes": [
    "repo:/Users/alvaro/src/foo",
    "user:alvaro",
    "global"
  ],
  "limit": 10
}
```

Retrieval should combine:

```text
FTS relevance
scope relevance
graph relationships
importance
recency
```

Results should include enough provenance for the model to reason about trustworthiness.

Example:

```text
content
memory_type
scope
created_at
source
score
```

---

# 16. MCP tool: forget

Purpose:

Remove or invalidate a memory.

Support:

```text
forget by memory id
```

Potential future modes:

```text
supersede
invalidate
archive
```

Hard deletion is acceptable for MVP.

Soft invalidation may be introduced later.

---

# 17. MCP tool: inspect

Purpose:

Allow the agent or developer to inspect a memory and its relationships.

Input:

```text
memory id
```

Output may include:

```text
memory
scopes
entities
incoming edges
outgoing edges
source metadata
```

---

# 18. Human CLI

The binary should not only expose MCP.

It should also provide a human-facing CLI for debugging and management.

Proposed commands:

```bash
memory mcp

memory remember "We use cargo nextest"

memory search "nextest"

memory list

memory show <id>

memory forget <id>

memory scopes

memory graph <id>

memory stats

memory export

memory doctor
```

Potential later commands:

```bash
memory import

memory vacuum

memory migrate

memory config
```

This CLI is important because the memory system must remain inspectable without an LLM.

---

# 19. Suggested Rust stack

## MCP

Use:

```text
rmcp
```

The MCP server should communicate over stdio.

---

## Async runtime

Use:

```text
tokio
```

Keep asynchronous code mostly at the MCP/process boundary.

Database access does not need to become unnecessarily asynchronous.

---

## SQLite

Use:

```text
rusqlite
```

with bundled SQLite.

Required SQLite capabilities:

```text
FTS5
foreign keys
recursive CTEs
WAL where appropriate
```

---

## CLI

Use:

```text
clap
```

---

## Serialization

Use:

```text
serde
serde_json
```

---

## IDs

Prefer:

```text
UUIDv7
```

using:

```text
uuid
```

UUIDv7 provides roughly time-ordered identifiers while remaining globally unique.

---

## Filesystem directories

Use:

```text
directories
```

Avoid manually assuming `$HOME`.

Default Unix-style location may resolve conceptually to:

```text
~/.memory/
```

but platform-native paths should be considered.

---

## Logging

Use:

```text
tracing
tracing-subscriber
```

Important:

MCP uses stdout for protocol communication.

Therefore application logs MUST NOT corrupt stdout.

Logs should go to:

```text
stderr
```

or an optional log file.

---

## Error handling

Use:

```text
thiserror
```

for structured application errors.

Use:

```text
anyhow
```

sparingly at application/CLI boundaries.

---

# 20. Suggested project structure

```text
src/
├── main.rs
│
├── cli/
│   ├── mod.rs
│   ├── remember.rs
│   ├── search.rs
│   ├── inspect.rs
│   └── doctor.rs
│
├── mcp/
│   ├── mod.rs
│   ├── server.rs
│   └── tools.rs
│
├── memory/
│   ├── mod.rs
│   ├── model.rs
│   ├── service.rs
│   ├── ranking.rs
│   └── scopes.rs
│
├── graph/
│   ├── mod.rs
│   └── traversal.rs
│
├── storage/
│   ├── mod.rs
│   ├── sqlite.rs
│   └── migrations.rs
│
├── search/
│   ├── mod.rs
│   └── fts.rs
│
└── config.rs
```

Prefer domain boundaries over excessive abstraction.

Do not create traits for components unless multiple implementations are actually needed.

---

# 21. Configuration

Default:

```text
~/.memory/config.toml
```

Possible initial options:

```toml
database = "~/.memory/memory.sqlite"

[search]
limit = 10

[ranking]
scope_weight = 1.0
recency_weight = 0.2
importance_weight = 0.3
```

Most users should not need to create this file.

Defaults should work without configuration.

---

# 22. Claude Code integration

Target experience:

```bash
claude mcp add memory -- memory mcp
```

Exact configuration syntax should be verified against the currently supported Claude Code MCP configuration.

The memory binary itself must not require Claude-specific behavior.

---

# 23. Codex CLI integration

Target conceptual configuration:

```toml
[mcp_servers.memory]
command = "memory"
args = ["mcp"]
```

Exact syntax should be verified against the current Codex configuration format.

The same MCP server must work for both clients.

---

# 24. Source/provenance

Memories should optionally record where they came from.

Potential sources:

```text
agent
human
import
repository
session
```

Suggested metadata:

```text
source_type
source_client
source_session
source_repository
created_by
```

This will become valuable later when ranking or debugging incorrect memories.

---

# 25. Memory deduplication

Do not build sophisticated semantic deduplication in MVP.

Initial strategy:

1. normalize content
2. calculate content hash
3. avoid identical duplicates inside the same scope

Later strategies may include:

```text
FTS similarity
vector similarity
entity overlap
LLM-based consolidation
```

These should be deferred until real duplicate patterns are observed.

---

# 26. Memory lifecycle

Initial lifecycle:

```text
created
    ↓
retrieved
    ↓
retrieved again
    ↓
optionally forgotten
```

Possible future lifecycle:

```text
observation
    ↓
reinforced
    ↓
durable knowledge
    ↓
superseded
```

Do not implement automatic consolidation until there is enough usage data to design it properly.

---

# 27. Security and privacy

The application is local-first.

Default behavior:

- no telemetry
- no network requests
- no hosted API dependency
- no automatic upload
- no external model calls

Permissions on the database/config directory should be restrictive where supported.

Secrets should not intentionally be stored.

Future versions may provide patterns or filters for obvious credential material.

---

# 28. Benefits

## Zero infrastructure

Users install one binary.

No:

```text
Docker
Neo4j
PostgreSQL
Redis
Python
Node.js
daemon
HTTP server
```

---

## Shared memory

The same memory store can be consumed by:

```text
Claude Code
Codex CLI
future MCP clients
human CLI
```

---

## Local-first

Everything remains on the developer's machine.

---

## Inspectable

Users can inspect, search, export and delete memories.

The memory system should never become an opaque black box.

---

## Graph-aware

Relationships can be modeled without requiring a graph database.

---

## Evolvable

SQLite leaves room for:

```text
FTS
vectors
graph traversal
metadata
migrations
analytics
```

without changing the deployment architecture.

---

# 29. Implementation phases

## Phase 0: Repository and foundation

- [x] Create Rust project
- [x] Define package/binary name
- [x] Add `clap`
- [x] Add `serde`
- [x] Add `serde_json`
- [x] Add `uuid` with UUIDv7 support
- [x] Add `directories`
- [x] Add `tracing`
- [x] Add `thiserror`
- [x] Add `rusqlite` with bundled SQLite
- [x] Add `rmcp`
- [x] Add `tokio`
- [x] Set up formatting
- [x] Set up Clippy
- [x] Set up tests
- [x] Add CI

### Done when

```text
cargo build
cargo test
cargo clippy
```

all succeed.

---

# 30. Phase 1: SQLite storage

- [x] Resolve application data directory
- [x] Create database automatically
- [x] Implement schema migrations
- [x] Create memories table
- [x] Create scopes table
- [x] Create memory/scopes relation
- [x] Create entities table
- [x] Create graph edges table
- [x] Create relevant indexes
- [x] Enable foreign keys
- [x] Add transaction helpers
- [x] Add database integration tests

### Done when

The application can reliably:

```text
create
read
update
delete
```

memories and relations.

---

# 31. Phase 2: Human CLI

Implement:

- [x] `graphmem remember`
- [x] `graphmem list`
- [x] `graphmem show`
- [x] `graphmem search`
- [x] `graphmem forget`
- [x] `graphmem scopes`
- [x] `graphmem doctor`

### Done when

The entire memory lifecycle can be exercised manually without MCP.

---

# 32. Phase 3: Full-text search

- [x] Enable FTS5
- [x] Create FTS index
- [x] Synchronize memory writes with FTS
- [x] Implement BM25 retrieval
- [x] Add scope filtering
- [x] Add BM25 ranking with deterministic tie-breakers
- [x] Add recency signal
- [x] Add importance signal
- [x] Add retrieval tests

Create a small retrieval benchmark dataset.

Example queries:

```text
How do we run integration tests?

Why don't we use repositories?

What did we learn about retrying jobs?

Which database convention does this project use?
```

---

# 33. Phase 4: MCP server

- [ ] Implement stdio MCP transport
- [ ] Implement server initialization
- [ ] Implement `remember`
- [ ] Implement `recall`
- [ ] Implement `forget`
- [ ] Implement `inspect`
- [ ] Validate tool schemas
- [ ] Ensure stdout contains MCP protocol only
- [ ] Send logs to stderr
- [ ] Add MCP integration tests

### Done when

Claude Code or another MCP inspector can:

```text
remember something
close the process
restart it
recall the memory
```

---

# 34. Phase 5: Client integration

## Claude Code

- [ ] Verify current MCP configuration
- [ ] Add documented setup
- [ ] Test `remember`
- [ ] Test `recall`
- [ ] Test multiple sessions
- [ ] Test repository-scoped memory

## Codex

- [ ] Verify current MCP configuration
- [ ] Add documented setup
- [ ] Test `remember`
- [ ] Test `recall`
- [ ] Test multiple sessions
- [ ] Test repository-scoped memory

---

# 35. Phase 6: Graph relationships

- [x] Implement entity creation
- [ ] Implement entity normalization
- [x] Implement edge creation
- [x] Implement incoming relationship lookup
- [x] Implement outgoing relationship lookup
- [ ] Implement recursive traversal
- [ ] Apply graph proximity to retrieval
- [ ] Add `graphmem graph` CLI inspection
- [ ] Add graph tests

Do not introduce automatic entity extraction yet unless clearly needed.

Initially relationships may be explicit or derived from structured `remember` calls.

---

# 36. Phase 7: Scope intelligence

- [ ] Detect current working directory
- [ ] Detect Git repository root
- [ ] Derive repository scope automatically
- [ ] Support user scope
- [ ] Support global scope
- [ ] Define scope precedence
- [ ] Include current scope automatically in MCP calls where appropriate
- [ ] Test same memory query across multiple repositories

Example:

```text
global
user:alvaro
repo:/Users/alvaro/src/foo
```

A query from `foo` should prioritize repository-specific memories before user/global memories.

---

# 37. Phase 8: Memory quality

Only after observing real usage:

- [ ] Detect exact duplicates
- [ ] Track memory access counts
- [ ] Track last access
- [ ] Experiment with importance
- [ ] Add superseding memories
- [ ] Add invalidation instead of deletion
- [ ] Evaluate automatic consolidation
- [ ] Build regression retrieval dataset

Do not add ML merely because it is available.

---

# 38. Phase 9: Optional semantic search

Only implement if FTS retrieval proves insufficient.

- [ ] Evaluate `sqlite-vec`
- [ ] Evaluate local embedding models
- [ ] Evaluate `fastembed`
- [ ] Add embedding table/index
- [ ] Implement hybrid lexical/vector search
- [ ] Benchmark against FTS-only retrieval
- [ ] Measure binary/build impact
- [ ] Make embeddings optional

Success criterion:

Semantic retrieval must demonstrate a meaningful improvement over:

```text
FTS
+
scope
+
graph
+
recency
```

before becoming part of the default architecture.

---

# 39. Phase 10: Distribution

- [ ] Produce Linux binaries
- [ ] Produce macOS binaries
- [ ] Produce Windows binaries if desired
- [ ] Add GitHub Releases
- [ ] Add install script
- [ ] Consider Homebrew formula
- [ ] Consider `cargo install`
- [ ] Document Claude Code setup
- [ ] Document Codex setup
- [ ] Document database location
- [ ] Document backup/export procedure

Target installation experience:

```bash
brew install memory
```

or:

```bash
cargo install memory
```

Then:

```bash
memory doctor
```

---

# 40. Testing strategy

Tests should focus on behavior rather than implementation details.

## Storage

- [x] migrations work from empty DB
- [x] foreign keys work
- [ ] duplicate handling works
- [x] transactions roll back correctly

## Search

- [x] obvious memory ranks first
- [ ] repo scope beats global scope
- [x] irrelevant memory ranks lower
- [x] forgotten memory is not returned

## Graph

- [x] incoming relationships work
- [x] outgoing relationships work
- [ ] bounded recursive traversal works
- [ ] cycles do not cause infinite traversal

## MCP

- [ ] valid MCP startup
- [ ] valid JSON schemas
- [ ] persistence across process restart
- [ ] logs never contaminate stdout

---

# 41. Engineering principles

## Keep the MCP surface small

Prefer:

```text
remember
recall
forget
inspect
```

over twenty database-oriented tools.

---

## SQLite is an implementation detail

MCP clients should never need to know the underlying database schema.

---

## Prefer explicit behavior first

Avoid premature autonomous extraction.

Start with deterministic memory operations.

---

## Optimize retrieval quality, not graph complexity

The graph exists to improve memory usefulness.

It is not the product itself.

---

## Keep memory inspectable

Every important memory should be inspectable and removable by the user.

---

## Preserve the single-binary property

New dependencies should be evaluated partly on whether they compromise:

```text
install binary → run binary
```

---

# 42. MVP definition

The MVP is complete when:

- [ ] the project produces one executable
- [ ] no external database is required
- [ ] no daemon is required
- [ ] memories persist under the user's local data directory
- [ ] memories support scopes
- [ ] SQLite FTS search works
- [ ] Claude Code can use it over MCP stdio
- [ ] Codex can use it over MCP stdio
- [ ] memories survive client/process restarts
- [ ] repository memories can override global/user memories
- [ ] the user can inspect memories from the CLI
- [ ] the user can delete memories
- [ ] logs cannot break the MCP stdio protocol

Graph traversal does **not** need to be sophisticated for MVP.

Vector search does **not** belong to MVP.

---

# 43. Initial dependency direction

Start with approximately:

```toml
[dependencies]
rmcp = { version = "*", features = ["server"] }

tokio = { version = "1", features = [
    "rt-multi-thread",
    "macros",
    "io-std"
] }

rusqlite = { version = "*", features = [
    "bundled"
] }

clap = { version = "4", features = ["derive"] }

serde = { version = "1", features = ["derive"] }
serde_json = "1"

uuid = { version = "1", features = [
    "v7",
    "serde"
] }

directories = "*"

tracing = "0.1"
tracing-subscriber = "*"

thiserror = "2"
anyhow = "1"
```

Before implementation, resolve current compatible versions.

Do not blindly copy these wildcard versions into the final package.

---

# 44. First implementation milestone

Build the smallest vertical slice:

```text
memory remember
        ↓
SQLite
        ↓
memory search
        ↓
MCP remember
        ↓
MCP recall
```

Do not implement graph reasoning first.

Recommended order:

```text
persistent memory
        ↓
useful retrieval
        ↓
MCP integration
        ↓
scopes
        ↓
graph enrichment
        ↓
semantic retrieval if necessary
```

A memory system with excellent storage and mediocre retrieval is useless.

A memory system with simple storage and excellent retrieval is valuable.

Optimize accordingly.
