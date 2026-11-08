#!/usr/bin/env bash
# Format + clippy sans avertissement.
. "$(dirname "$0")/lib.sh"
cargo_do -- "rustup component add rustfmt clippy >/dev/null 2>&1 && cargo fmt --all --check && cargo clippy --locked --all-targets -- -D warnings"
echo "fmt-clippy: OK"
