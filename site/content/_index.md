---
title: Graphmem documentation
---

# Shared memory for coding agents

![Graphmem — Shared, local memory for coding agents](docs/assets/graphmem-banner.png)

Keep project decisions across sessions and tools. Graphmem stores memories
locally, finds them by meaning, and links them through a knowledge graph.
Optional code tools help agents navigate your repository.

## Get started

```sh
npm install --global @sonic182/graphmem
gmem-install
gmem version
```

Then [install the plugin for your coding agent](docs/plugins.md).
Pi can handle binary setup on startup; OpenCode prints an installer command
if needed. Package installation itself downloads no binary.

## Explore

- [CLI reference](docs/cli.md) — store, search, and browse memories.
- [MCP reference](docs/mcp.md) — connect another client and use the tools.
- [Build and configuration](docs/setup.md) — source builds, CUDA, and tuning.
- [Design](docs/design.md) — storage, retrieval, and architecture.
- [Code-tool benchmark](docs/evaluation/code-tools-agent-benchmark.md) — results and limitations.
