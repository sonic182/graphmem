# Development

Use the CLI feedback loop instead of an editor or LSP integration.

## Feedback loop

- Use `rg` for plain-text searches and `ast-grep` for syntax-aware code searches whenever available; use the next-best fallback only when they are unavailable.
- Start with package-scoped checks: `just check`, `just test`, and `just lint`.
- Run `just fmt` after edits.
- Run `just verify` before handing off changes.
- Use `just ra` for supplemental rust-analyzer diagnostics when needed.
- Keep MCP protocol output on stdout; application logs belong on stderr.

## Local setup

The repository toolchain configuration installs stable Rust with rustfmt,
Clippy, and rust-analyzer through rustup. Install the external Cargo tools once:

```bash
cargo install --locked cargo-nextest
cargo install --locked just
```

Optional human feedback loop:

```bash
cargo install --locked bacon
bacon clippy
```

Do not add an LSP bridge or `rust-analyzer-cli` unless ordinary compiler and
rust-analyzer diagnostics prove insufficient for a concrete task.
