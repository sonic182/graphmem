# Editor plugins

Plugins add Graphmem's memory and code-navigation skills, session guidance, and
MCP registration.

Claude Code and Codex need [`gmem` installed](../README.md#install) and
Node.js on `PATH`. Pi and OpenCode can also use a package-local binary.
Installing the npm package or loading a plugin does not download a binary.

## Claude Code

```sh
claude plugin marketplace add sonic182/graphmem
claude plugin install graphmem@graphmem
```

Alternatively, inside Claude Code, open `/plugins`, go to the marketplace menu,
and add `sonic182/graphmem`. Then select `graphmem` in the plugin list and install it.

Check `/mcp` for the `gmem` server. If missing:

```sh
claude mcp add gmem -- gmem mcp
```

To keep Graphmem as your only memory store, add this to your shell profile:

```sh
export CLAUDE_CODE_DISABLE_AUTO_MEMORY=1
```

Update the binary separately and refresh the plugin with
`claude plugin marketplace update graphmem`.

<details>
<summary>Install from a local checkout</summary>

```sh
cargo install --locked --features code --path .
claude plugin marketplace add ./
claude plugin install graphmem@graphmem
```

Keep the `./` prefix. Re-run Cargo installation after pulling binary changes.

</details>

## Codex

```sh
codex plugin marketplace add sonic182/graphmem
codex plugin add graphmem@graphmem
```

Review and trust the Graphmem hooks in `/hooks`, then start a new thread.
Check `/mcp` for `gmem`; if missing:

```sh
codex mcp add gmem -- gmem mcp
```

<details>
<summary>Install from a local checkout</summary>

```sh
just install-codex-plugin
```

Use `just install-codex-plugin true` for CUDA; `nvcc` must be on `PATH`.

</details>

## OpenCode

```sh
opencode plugin @sonic182/graphmem --global
```

If no binary is available, run the absolute installer command printed by the
plugin, then restart OpenCode. The command uses Node.js 18+; `bun` also works.
Check connectivity with `opencode mcp list`.

Existing user-defined `gmem` entries, including disabled entries, are preserved.
Guidance is added only when the server is connected.

<details>
<summary>Install from GitHub or a local checkout</summary>

```sh
opencode plugin graphmem@git+https://github.com/sonic182/graphmem.git#master --global
```

Use this explicit URL form; the `github:` shorthand has cache/path issues.
Replace `master` with a commit to pin a version; add `--force` to update.

From a checkout:

```sh
cargo install --locked --features code --path .
opencode plugin ./ --global
```

Restart OpenCode after plugin changes.

</details>

## pi

```sh
pi install npm:@sonic182/graphmem
```

On startup, Pi reuses an existing binary or offers **Download**, **Use existing
executable**, or **Not now**. Downloads are approved explicitly, verified, and
cached by package version. Unsupported platforms need a source build.

To retry setup or choose an executable:

```text
/graphmem-setup
/graphmem-setup /absolute/path/to/gmem
```

Check connectivity with `/mcp`. Registration is session-only:
`pi mcp list` does not load extensions and cannot see it. Existing user/project
MCP entries, including disabled entries, are preserved.

After updates, restart Pi or run `/reload`. Approve the new version's binary
download or keep using your native executable. Uninstall with
`pi remove npm:@sonic182/graphmem`.

<details>
<summary>Migration, headless setup, and manual registration</summary>

Remove `pi-mcp-adapter` if installed; its `/mcp` overrides Pi's native support:

```sh
pi remove npm:pi-mcp-adapter
```

For other installation sources, remove the corresponding package or extension.
Move server entries from `~/.config/mcp/mcp.json` to Pi's native configuration;
native MCP does not read the adapter's file.

Headless modes print an absolute installer command instead of prompting.
Run it and restart Pi.

For persistent registration, install `gmem` and run:

```sh
pi mcp add gmem -- /absolute/path/to/gmem mcp
```

Add `--local` for project configuration. To install the package from a checkout,
run `pi install ./`.

</details>
