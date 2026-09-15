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

ra:
    rust-analyzer diagnostics .

verify: fmt-check check lint test
