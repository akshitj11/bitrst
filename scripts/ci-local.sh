#!/usr/bin/env bash
set -euo pipefail

export RUSTFLAGS="-D warnings"

echo "== fmt =="
cargo fmt --check

echo "== clippy =="
cargo clippy --locked --all-targets --all-features --features test-short-period -- -D warnings

echo "== test (fast) =="
cargo test --locked --all --features test-short-period

echo "== test (full interval) =="
cargo test --locked --all

echo "== audit =="
cargo audit

echo "== deny =="
cargo deny check advisories
cargo deny check bans
cargo deny check licenses
cargo deny check sources

echo "== build release =="
cargo build --locked --release --workspace

echo "ALL LOCAL CI CHECKS PASSED"
