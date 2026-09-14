# Development

Use the CLI feedback loop instead of an editor or LSP integration.

## Feedback loop

- Use `rg` for plain-text searches and `ast-grep` for syntax-aware code searches whenever available; use the next-best fallback only when they are unavailable.
- Start with package-scoped checks: `just check`, `just test`, and `just lint`.
- Run `just fmt` after edits.
- Run `just verify` before handing off changes.
- Use `just ra` for supplemental rust-analyzer diagnostics when needed.
- Keep MCP protocol output on stdout; application logs belong on stderr.

## Architecture

Use a small layered architecture with a hexagonal boundary where it pays for itself:

- `src/domain.rs` contains domain data and invariants; it must not depend on Clap, rusqlite, or transport code.
- `src/infrastructure/` contains external adapters such as SQLite schema setup and filesystem access.
- `src/application.rs` is introduced when a use case coordinates multiple domain or infrastructure operations.
- `src/cli.rs` is introduced with the human CLI; it parses arguments, formats output, and delegates behavior to application code.
- `src/main.rs` is the composition root for concrete adapters and process exit behavior.

Do not add ports, repository traits, factories, events, or dependency-injection machinery until a second adapter or a real testing boundary requires one. Prefer integration tests for SQLite and CLI behavior; add unit tests only for pure domain logic.

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
