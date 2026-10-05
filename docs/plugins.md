# Editor plugins

Graphmem ships its skills, lifecycle guidance, and MCP server configuration to Claude Code, Codex, OpenCode, and pi.

The npm package ships no binaries and runs no installation scripts. For the
standalone CLI, Claude Code, or Codex, run `npm install --global @sonic182/graphmem`
and then `gmem-install`, or [install a prebuilt binary](../README.md#install) or
build with Cargo. The explicit installer reuses a working native `gmem` on your
`PATH`; otherwise it downloads and verifies the package version's CPU binary.
It never replaces a separately installed binary.

Pi and OpenCode can also use their package-local binary without changing your
`PATH`. Pi asks before downloading; OpenCode provides an explicit installer
command. No download occurs during package installation or plugin loading.
Cargo installs `gmem` at `$HOME/.cargo/bin/gmem`; for manual MCP registration,
you can use that absolute path instead of `gmem`. Claude Code and Codex hooks
need `node`. OpenCode loads the plugin with Bun; its recovery command requires
Node.js 18+ or can be run with `bun` instead of `node`.

| Harness | What the plugin provides | MCP registration |
| --- | --- | --- |
| Claude Code | skill + `SessionStart`/`SubagentStart` hooks + MCP server | bundled (`.mcp.json`) |
| Codex | skill + lifecycle hooks + MCP server | bundled (`.mcp.json`) |
| OpenCode | skill + session guidance + MCP server | plugin `config` hook |
| pi | skill + session-start guidance and consent extension | session-local native MCP registration; existing config wins |

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

Install the npm plugin:

```sh
opencode plugin @sonic182/graphmem --global
```

The plugin adds both Graphmem skills through `skills.paths`. It reuses a working
native `gmem` or its package-local cache, registers the MCP server using the
absolute executable path, and injects guidance only when the server is connected.
Existing user-defined `gmem` entries, including disabled entries, are unchanged.

If no binary is available, OpenCode prints one recovery message containing an
absolute installer command. Run that command only if you approve the download,
then restart OpenCode and check `opencode mcp list`. For example (replace the
placeholder with the actual path shown in the message):

```sh
node '/absolute/package/path/scripts/npm-install.js'
```

There is no automatic download or blocking confirmation prompt during plugin
loading. If Node.js is unavailable, use `bun` with the same script path.

To install directly from GitHub instead (no npm publication needed):

```sh
opencode plugin graphmem@git+https://github.com/sonic182/graphmem.git#master --global
```

Use the explicit `graphmem@git+https://...` form rather than the `github:` shorthand,
which has known cache/path-resolution issues.

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

Pi has built-in MCP support. If you previously installed `pi-mcp-adapter`,
remove it before switching: its `/mcp` command overrides Pi's native MCP support,
even when `pi mcp list` reports a successful connection.

```sh
pi remove npm:pi-mcp-adapter
```

If you installed the adapter from another source or loaded it manually, remove
that package source or extension entry instead. Re-register servers from the
adapter's `~/.config/mcp/mcp.json` in Pi's native configuration; the old file is
not read by native MCP support.

Install Graphmem's package:

```sh
pi install npm:@sonic182/graphmem
```

On the next interactive startup, the extension reuses a working native `gmem` or
its package-local cache. If neither exists, it offers **Download**, **Use existing
executable**, or **Not now**. Download selects the matching platform and package
version, verifies `SHA256SUMS`, smoke-tests the executable, and caches it. No
network activity occurs before approval. Unsupported platforms need a source
build or an existing compatible executable.

**Not now** defers setup for the session without repeated prompts. Run
`/graphmem-setup` to retry, or `/graphmem-setup /absolute/path/to/gmem` to explicitly
select a trusted executable. Headless modes, including RPC, never wait for
approval input: they print an absolute installer command. Run it explicitly and
restart Pi, or invoke `/graphmem-setup` after installation.

The package adds both skills and registers `gmem mcp` for the current session
using the resolved absolute executable path. It does not write `mcp.json`.
Existing user configuration in `~/.pi/agent/mcp.json`, trusted project entries in
`.pi/mcp.json`, and other extensions' `gmem` registrations are preserved, even
when disabled. Use `/mcp` to check actual connectivity; shell-level `pi mcp list`
does not load extensions and therefore cannot see session-only registrations.
Guidance is injected only after MCP tools become available.

For persistent manual registration instead, install `gmem` first and run:

```sh
pi mcp add gmem -- /absolute/path/to/gmem mcp
```

This writes user-level configuration; add `--local` for the current project.

To install the package from a local checkout instead:

```sh
pi install ./
```

Restart Pi after package updates or run `/reload`. A new package version needs
its own cached binary, so approve the new download or keep using your existing
native executable. Check `/mcp` for connectivity and `pi list` for the package.
Failed downloads can be retried with `/graphmem-setup`; failures do not leave a
partially installed executable. Linux release binaries require glibc 2.39 and
OpenSSL 3; CUDA and other unsupported systems need a source build.
Uninstall the package with:

```sh
pi remove npm:@sonic182/graphmem
```
