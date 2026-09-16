default: verify

check package="graphmem":
    cargo check -p {{package}} --all-targets --message-format=short

lint package="graphmem":
    cargo clippy -p {{package}} --all-targets --message-format=short -- -D warnings

test package="graphmem":
    cargo nextest run -p {{package}} --no-fail-fast --no-tests=pass

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

install-codex-plugin cuda="false":
    if ! codex mcp get gmem >/dev/null 2>&1; then if [ "{{cuda}}" = "true" ]; then cargo install --path . --features cuda; else cargo install --path .; fi; codex mcp add gmem -- gmem mcp; fi
    codex plugin marketplace add .
    codex plugin add graphmem@graphmem

ra:
    rust-analyzer diagnostics .

verify: fmt-check check lint test
