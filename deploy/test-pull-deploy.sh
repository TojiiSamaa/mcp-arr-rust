#!/usr/bin/env bash
# Simulation de pull-deploy.sh avec de FAUSSES commandes docker/curl (PATH) : succès, déjà à jour, rollback, sans précédent.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
W="$(mktemp -d)"; trap 'rm -rf "$W"' EXIT
mkdir -p "$W/bin" "$W/state"
cat > "$W/bin/docker" <<'F'
#!/usr/bin/env bash
# état simulé : $FAKE/running = référence en service ; FAKE_PULLED = digest de :prod
case "$1" in
  pull) exit 0 ;;
  image) echo "ghcr.io/tojiisamaa/mcp-arr-rust@sha256:${FAKE_PULLED}" ;;
  inspect) cat "$FAKE/running" ;;
  compose)
    case "$*" in
      *" ps -q"*) echo cid ;;
      *" up -d"*) echo "$ARR_IMAGE" > "$FAKE/running"; echo "$ARR_IMAGE" >> "$FAKE/ups" ;;
    esac ;;
esac
F
cat > "$W/bin/curl" <<'F'
#!/usr/bin/env bash
# sain tant que la référence en service ne contient pas FAKE_BAD
if grep -q "${FAKE_BAD:-@@none@@}" "$FAKE/running" 2>/dev/null; then exit 22; fi
echo '{"status":"ok"}'
F
chmod +x "$W/bin/"*
export FAKE="$W" PATH="$W/bin:$PATH" STATE_DIR="$W/state" COMPOSE_FILE=/x/c.yml HEALTH_WAIT=0 HEALTH_TRIES=2
I=ghcr.io/tojiisamaa/mcp-arr-rust
fail=0
check() { if [ "$2" = "$3" ]; then echo "ok   $1"; else echo "FAIL $1 (attendu $2, obtenu $3)"; fail=1; fi; }
run() { "$HERE/pull-deploy.sh" "$@" >"$W/out" 2>&1; echo $?; }

echo "$I@sha256:old" > "$W/running"; echo "$I@sha256:old" > "$W/state/current"; : > "$W/ups"

check "a-blanc ne touche à rien" 0 "$(FAKE_PULLED=new run --a-blanc)"
check "a-blanc : aucun up" 0 "$(wc -l < "$W/ups")"

check "succès (old -> new)" 0 "$(FAKE_PULLED=new run)"
check "current = new" "$I@sha256:new" "$(cat "$W/state/current")"
check "previous = old" "$I@sha256:old" "$(cat "$W/state/previous")"

: > "$W/ups"
check "déjà à jour : code 0" 0 "$(FAKE_PULLED=new run)"
check "déjà à jour : aucun up" 0 "$(wc -l < "$W/ups")"

check "échec de santé -> rollback (code 1)" 1 "$(FAKE_PULLED=bad FAKE_BAD=sha256:bad run)"
check "service revenu sur new" "$I@sha256:new" "$(cat "$W/running")"
check "current inchangé = new" "$I@sha256:new" "$(cat "$W/state/current")"
grep -q ROLLBACK "$W/out" && echo "ok   journal mentionne ROLLBACK" || { echo "FAIL journal"; fail=1; }

# rollback lui-même en échec : tout est malsain
check "rollback échoué (code 2)" 2 "$(FAKE_PULLED=bad FAKE_BAD=sha256: run)"

# premier déploiement sans état ni précédent, nouvelle version malsaine
rm -f "$W/state/current" "$W/state/previous"; : > "$W/running"
check "sans précédent : code 3" 3 "$(FAKE_PULLED=bad FAKE_BAD=sha256:bad run)"
exit "$fail"
