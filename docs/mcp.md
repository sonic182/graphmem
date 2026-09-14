# MCP stdio

Run `graphmem mcp` as an MCP server over newline-delimited JSON-RPC. The
database is the same SQLite database used by the CLI and is selected with
`GRAPHMEM_HOME`.

The server exposes exactly four tools:

- `remember`: stores `content`, with optional `memory_type`, `importance`, and
  `scopes`. Defaults are `observation`, `0.0`, and `global`.
- `recall`: searches `query` with optional `scopes` and `limit` (default 10).
  Without scopes it searches global memories. A repository scope also includes
  global memories, with repository matches first.
- `inspect`: returns a memory and its scopes by `id`.
- `forget`: deletes a memory by `id`.

Scopes are explicit: use `global` for reusable knowledge and
`repo:/absolute/path/to/repository` for project-specific knowledge. The server
does not infer a repository from the working directory.

Example configuration:

```json
{
  "mcpServers": {
    "graphmem": {
      "command": "graphmem",
      "args": ["mcp"]
    }
  }
}
```
