#!/usr/bin/env bash
# Tests unitaires (lib + binaire) puis bout en bout stdio ET HTTP (tests/e2e.rs).
. "$(dirname "$0")/lib.sh"
cargo_do -- "cargo test --locked --lib --bins && cargo test --locked --test e2e --test securite_fermee"
echo "test: OK"
