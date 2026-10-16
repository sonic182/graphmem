default: verify

check package="graphmem" features="code":
    cargo check -p {{package}} --all-targets --features "{{features}}" --message-format=short

lint package="graphmem" features="code":
    cargo clippy -p {{package}} --all-targets --features "{{features}}" --message-format=short -- -D warnings

test package="graphmem" features="code":
    cargo nextest run -p {{package}} --features "{{features}}" --no-fail-fast --no-tests=pass

# The default build, without optional features, must keep compiling.
check-lean package="graphmem":
    cargo check -p {{package}} --all-targets --message-format=short

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

install-codex-plugin cuda="false":
    #!/usr/bin/env bash
    set -euo pipefail
    if [ "{{cuda}}" = "true" ]; then
        cargo install --path . --features cuda
    else
        cargo install --path .
    fi
    codex plugin marketplace list | grep -q '^graphmem' || codex plugin marketplace add .
    codex plugin list | grep -q 'graphmem@graphmem' || codex plugin add graphmem@graphmem

ra:
    rust-analyzer diagnostics .

migrate package="graphmem":
    cargo run -p {{package}} --quiet -- migrate

verify: fmt-check check check-lean lint test
