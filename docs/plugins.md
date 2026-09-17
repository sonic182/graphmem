# Editor plugins

Graphmem ships its skills, lifecycle guidance, and MCP server configuration to
Claude Code, Codex, and pi.

All flows need `gmem` on your `PATH`. The Claude Code and Codex hooks also need
`node`; without it the MCP server still works, but the agent does not receive
the recall/store guidance.

| Harness | What the plugin provides | MCP registration |
| --- | --- | --- |
| Claude Code | skill + `SessionStart`/`SubagentStart` hooks | explicit (`claude mcp add`) |
| Codex | skill + lifecycle hooks | explicit (`codex mcp add`, or the `just` recipe) |
| pi | skill + session-start guidance extension + MCP server config | bundled (`pi-mcp-adapter`) |

## Claude Code

From a local checkout, install the optimized binary from it and register it,
then add the checkout itself as the marketplace:

```sh
cargo install --locked --path .
claude mcp add gmem -- "$HOME/.cargo/bin/gmem" mcp
claude plugin marketplace add ./
claude plugin install graphmem@graphmem
```

Avoid registering `target/debug/gmem`: a debug build runs the embedding model
far too slowly for everyday use. Re-run `cargo install --locked --path .` after
pulling changes (the registered path stays the same), and `claude plugin
marketplace update graphmem` after changing the plugin manifest or hooks. The
marketplace source needs the `./` prefix.

To install from GitHub instead:

```sh
cargo install --locked --git https://github.com/sonic182/graphmem
claude mcp add gmem -- "$HOME/.cargo/bin/gmem" mcp
claude plugin marketplace add sonic182/graphmem
claude plugin install graphmem@graphmem
```

The plugin bundles the skills and the `SessionStart`/`SubagentStart` hooks. It
does not bundle the MCP server: the `mcpServers` manifest field is unreliable
in current Claude Code, so the MCP is registered explicitly above.

## Codex

From a local Graphmem checkout, install the binary and plugin with one command:

```sh
just install-codex-plugin
```

For a CUDA build, use:

```sh
just install-codex-plugin true
```

The CUDA build requires `nvcc` on your `PATH`.

The recipe installs `gmem` and registers its MCP server only when `gmem` is
not already configured; the optional `true` adds Cargo's `cuda` feature for that new
install. It then adds the checkout as a Codex marketplace and installs
`graphmem`. An existing `gmem` configuration is left unchanged. Open `/hooks`
in Codex, review and trust the two Graphmem hooks, then start a new thread.

To install the plugin from GitHub, install `gmem`, register it as an MCP
server, and then install the plugin:

```sh
cargo install --locked --git https://github.com/sonic182/graphmem
codex mcp add gmem -- "$HOME/.cargo/bin/gmem" mcp
codex plugin marketplace add sonic182/graphmem
codex plugin add graphmem@graphmem
```

The plugin uses Codex's `gmem mcp` server, so semantic calls reuse the same
in-memory embedding model. The lifecycle hooks require `node` on your `PATH`;
without it, the MCP server still works but Codex does not receive the
recall/store guidance.

## pi

pi has no built-in MCP support, so this package pairs with
[`pi-mcp-adapter`](https://www.npmjs.com/package/pi-mcp-adapter). The package
bundles the skill, a session-start guidance extension, and the `gmem mcp`
server config, so there is no separate MCP registration step.

From a local checkout:

```sh
cargo install --locked --path .
pi install npm:pi-mcp-adapter   # once, if you do not already use it
pi install ./
```

From GitHub:

```sh
cargo install --locked --git https://github.com/sonic182/graphmem
pi install npm:pi-mcp-adapter
pi install git:github.com/sonic182/graphmem
```

The package registers `gmem mcp` lazily with direct tools and injects the
recall/store guidance before the first turn. Restart pi after installing, or
run `/reload`.

`pi -e <source>` loads the skill and the guidance but not the MCP server: the
adapter only loads MCP servers from packages listed in settings, so use
`pi install` when you want the tools. Use `pi list` to confirm the package and
`pi remove <source>` (the same source you installed) to uninstall it.
