#!/bin/sh
# Preuve B sous isolement réseau du système : le conteneur n'a AUCUNE interface (hors boucle locale),
# donc même un défaut du coupe-circuit ne pourrait rien faire sortir. Mêmes commandes en local et en CI.
set -eu
cd "$(dirname "$0")/../.."
docker run --rm --network none \
  -v cargo-reg:/usr/local/cargo/registry -v arr-target:/target -e CARGO_TARGET_DIR=/target \
  -v "$PWD":/src -w /src rust:1-alpine@sha256:44819b27be4c97b0e4d7203bd2e02c4820f97700211b6a37fb4c246a2de59383 \
  sh -c 'apk add --no-cache musl-dev >/dev/null && cargo test --offline --test proof_ab'
