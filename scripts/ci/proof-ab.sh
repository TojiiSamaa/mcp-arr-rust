#!/usr/bin/env bash
# Preuve A (statique), mutation de la preuve A (DOIT échouer), preuve B sans réseau :
# docker --network none ET unshare -n. Compile avec réseau, exécute sans.
. "$(dirname "$0")/lib.sh"
out="$(cd "$REPO/.." && pwd)/.arr-ci-tmp.$$"; mkdir -p "$out"; trap 'rm -rf "$out"' EXIT

echo "== preuve A (statique)"
cargo_do -- "cargo test --locked --test proof_ab -- proof_a_ mutation_a"

echo "== mutation de la preuve A : un second point réseau injecté dans une COPIE doit faire échouer"
cp -r "$REPO"/. "$out/mut" && rm -rf "$out/mut/.git"
printf 'pub fn mutation_egress() { let _ = reqwest::Client::new(); }\n' > "$out/mut/src/zz_mutation.rs"
if docker run --rm -v cargo-reg:/usr/local/cargo/registry -v arr-target:/target -e CARGO_TARGET_DIR=/target \
     -v "$out/mut":/src -w /src "$RUST_IMAGE" sh -c "apk add -q --no-cache musl-dev >/dev/null && cargo test --locked --test proof_ab -- proof_a_" >"$out/mut.log" 2>&1; then
  tail -20 "$out/mut.log"; echo "ECHEC: la preuve A a ACCEPTE la mutation"; exit 1
fi
grep -q "network code outside client.rs" "$out/mut.log" || { tail -20 "$out/mut.log"; echo "ECHEC: échec pour une autre raison"; exit 1; }
echo "mutation: la preuve A a échoué pour la bonne raison :"; grep -m1 "network code outside" "$out/mut.log"

FILTER="proof_b approver_scenarios no_tool_can_toggle"
echo "== preuve B : binaire de test compilé avec réseau, exécuté sans"
bin=$(cargo_do -- "cargo test --locked --test proof_ab --no-run --message-format=json" \
  | grep -o '"executable":"[^"]*proof_ab[^"]*"' | tail -1 | cut -d'"' -f4)
[ -n "$bin" ] || { echo "binaire proof_ab introuvable"; exit 1; }
docker run --rm --user "$(id -u):$(id -g)" -v arr-target:/target -v "$out":/out alpine:3@sha256:3f4d6b32e26703c6082839234f43769d5046b3ac7c910033911d1e71d8cd630a cp "$bin" /out/proof_ab
chmod +x "$out/proof_ab"

echo "-- docker --network none"
docker run --rm --network none -v "$out/proof_ab":/proof_ab:ro -v "$REPO":/src:ro -w /src alpine:3@sha256:3f4d6b32e26703c6082839234f43769d5046b3ac7c910033911d1e71d8cd630a /proof_ab $FILTER
echo "-- unshare -n"
U=()
if unshare -n true 2>/dev/null; then U=(unshare -n)
elif command -v sudo >/dev/null && sudo -n unshare -n true 2>/dev/null; then U=(sudo -n unshare -n); fi
if [ ${#U[@]} -gt 0 ]; then ( cd "$REPO" && "${U[@]}" "$out/proof_ab" $FILTER )
elif [ -n "${GITHUB_ACTIONS:-}" ]; then echo "ECHEC: unshare -n indisponible sur le runner"; exit 1
else echo "SKIP local: unshare -n non permis ici (la CI l'exécute)"; fi
echo "proof-ab: OK"
