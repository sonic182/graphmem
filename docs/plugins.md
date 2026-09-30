# Editor plugins

Graphmem ships its skills, lifecycle guidance, and MCP server configuration to Claude Code, Codex, OpenCode, and pi.

All flows need `gmem` on your `PATH`. [Install a prebuilt binary](../README.md#install) first (or build with Cargo). Cargo installs `gmem` at `$HOME/.cargo/bin/gmem`; if that directory is not on your editor's `PATH`, add it there. For the manual `mcp add` fallbacks below, you can instead replace `gmem` with `"$HOME/.cargo/bin/gmem"`. The Claude Code and Codex hooks also need `node`; without it the MCP server still works, but the agent does not receive the recall/store guidance. OpenCode runs the plugin on Bun and needs no extra runtime.

| Harness | What the plugin provides | MCP registration |
| --- | --- | --- |
| Claude Code | skill + `SessionStart`/`SubagentStart` hooks + MCP server | bundled (`.mcp.json`) |
| Codex | skill + lifecycle hooks + MCP server | bundled (`.mcp.json`) |
| OpenCode | skill + session guidance + MCP server | plugin `config` hook |
| pi | skill + session-start guidance extension | shared MCP config (`pi-mcp-adapter`) |

## Claude Code

With `gmem` installed, add the repository as a plugin marketplace and install the plugin:

```sh
claude plugin marketplace add sonic182/graphmem

# install the Graphmem plugin (skill + hooks + MCP server)
claude plugin install graphmem@graphmem
```

The plugin provides three things: the `graphmem-mcp-for-dev` and `graphmem-code-analysis` skills, the `SessionStart`/`SubagentStart` hooks that inject the recall/store and code-navigation guidance into every session and subagent, and the `gmem mcp` server through its bundled `.mcp.json`, which exposes the `remember`/`recall`/`update`/`relate`/`graph`/`inspect`/`forget` tools. Nothing else is needed.

After installing, run `/mcp` and confirm the `gmem` server is listed. Claude Code has open bugs where a plugin's `.mcp.json` is not copied into the plugin cache, so if it is missing, register the server explicitly:

```sh
claude mcp add gmem -- gmem mcp
```

Replace the binary after new releases (or re-run `cargo install` if installed from source), and run `claude plugin marketplace update graphmem` after the plugin manifest, hooks, or `.mcp.json` change.

To install from a local checkout instead:

```sh
cargo install --locked --path .
claude plugin marketplace add ./
claude plugin install graphmem@graphmem
```

The marketplace source needs the `./` prefix. Re-run `cargo install --locked --path .` after pulling changes.

Graphmem replaces Claude Code's built-in auto-memory, so disable the latter to keep one memory store. Add this to your shell's rc file (`.bashrc`, `.zshrc`, `~/.config/fish/config.fish`, or equivalent):

```sh
export CLAUDE_CODE_DISABLE_AUTO_MEMORY=1
```

## Codex

With `gmem` installed, add the repository as a plugin marketplace and install the plugin:

```sh
codex plugin marketplace add sonic182/graphmem

# install the Graphmem plugin (skill + hooks + MCP server)
codex plugin add graphmem@graphmem
```

The plugin provides the `graphmem-mcp-for-dev` and `graphmem-code-analysis` skills, the lifecycle hooks, and the `gmem mcp` server through its bundled `.mcp.json`. After installing, run `/mcp` and confirm the `gmem` server is listed; if it is missing, register it explicitly:

```sh
codex mcp add gmem -- gmem mcp
```

Open `/hooks` in Codex, review and trust the two Graphmem hooks, then start a new thread.

To install from a local Graphmem checkout instead, install the binary and plugin with one command:

```sh
just install-codex-plugin
```

For a CUDA build, use:

```sh
just install-codex-plugin true
```

The CUDA build requires `nvcc` on your `PATH`. The optional `true` adds Cargo's `cuda` feature to the install. The recipe then adds the checkout as a Codex marketplace and installs `graphmem`.

The plugin uses Codex's `gmem mcp` server, so semantic calls reuse the same in-memory embedding model. The lifecycle hooks require `node` on your `PATH`; without it, the MCP server still works but Codex does not receive the recall/store guidance.

## OpenCode

With `gmem` installed, install the plugin directly from GitHub (no npm publication needed):

```sh
opencode plugin graphmem@git+https://github.com/sonic182/graphmem.git#master --global
```

The plugin provides three things: the `graphmem-mcp-for-dev` and `graphmem-code-analysis` skills (registered through `skills.paths`), the recall/store and code-navigation guidance injected into the system prompt (the memory part is also preserved across compaction), and the `gmem mcp` server registered through the plugin `config` hook. An existing user-defined `gmem` MCP entry is left unchanged. Use the explicit `graphmem@git+https://...` form rather than the `github:` shorthand, which has known cache/path-resolution issues.

Pin to a commit for reproducibility, and re-run with `--force` to update:

```sh
opencode plugin graphmem@git+https://github.com/sonic182/graphmem.git#<commit> --global --force
```

To install from a local checkout instead:

```sh
cargo install --locked --path .
opencode plugin ./ --global
```

Restart OpenCode after installing or changing the plugin, then run `opencode mcp list` and confirm the `gmem` server is listed.

## pi

pi has no built-in MCP support, so install [`pi-mcp-adapter`](https://www.npmjs.com/package/pi-mcp-adapter) and configure `gmem mcp` once in pi's shared MCP config. The Graphmem package itself only adds its skills and session-start guidance.

With `gmem` installed, set up pi:

```sh
pi install npm:pi-mcp-adapter   # once, if you do not already use it
pi install git:github.com/sonic182/graphmem
mkdir -p ~/.config/mcp
```

To install from a local checkout instead, build the binary and replace the `pi install` command with:

```sh
cargo install --locked --path .
pi install ./
```

Create `~/.config/mcp/mcp.json`:

```json
{
  "mcpServers": {
    "gmem": {
      "command": "/absolute/path/to/gmem",
      "args": ["mcp"],
      "lifecycle": "lazy",
      "directTools": true,
      "toolPrefix": "none"
    }
  }
}
```

Restart pi after installing or changing the config, or run `/reload` then `/mcp reconnect gmem`. `pi -e <source>` still loads the skill and guidance for quick testing, but MCP setup remains in the shared config. Use `pi list` to confirm the package and `pi remove <source>` (the same source you installed) to uninstall it.
