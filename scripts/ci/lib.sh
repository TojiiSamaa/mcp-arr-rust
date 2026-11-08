# Sourcé par les autres scripts : mêmes commandes en local et sur GitHub (Docker requis).
set -euo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
RUST_IMAGE="${RUST_IMAGE:-rust:1-alpine@sha256:44819b27be4c97b0e4d7203bd2e02c4820f97700211b6a37fb4c246a2de59383}"

# cargo_do [options docker] -- <commande sh>
# Compile dans un conteneur : registre et cible en volumes nommés (cache entre exécutions).
cargo_do() {
  local opts=()
  while [ "$1" != "--" ]; do opts+=("$1"); shift; done
  shift
  docker run --rm "${opts[@]}" \
    -v cargo-reg:/usr/local/cargo/registry -v arr-target:/target -e CARGO_TARGET_DIR=/target \
    -v "$REPO":/src -w /src "$RUST_IMAGE" \
    sh -c "apk add -q --no-cache musl-dev >/dev/null && $*"
}
