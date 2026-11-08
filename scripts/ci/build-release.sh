#!/usr/bin/env bash
# Build release et contrôle : le binaire démarre et liste ses outils.
. "$(dirname "$0")/lib.sh"
cargo_do -- "cargo build --release --locked && /target/release/arrmcp --version && /target/release/arrmcp --list-tools | head -3"
echo "build-release: OK"
