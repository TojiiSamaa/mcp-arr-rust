#!/usr/bin/env bash
# Mise en prod SORTANTE de arrmcp : c'est l'hôte qui tire l'image, rien n'est poussé vers lui (aucun port entrant,
# aucun secret de déploiement). Lancé par deploy/pull-deploy.timer.
#   1. tire IMAGE:$TAG (latest par défaut ; publié par release-arr.yml avec le tag de version, à la release) et lit son digest ;
#   2. si c'est le digest déjà en service et sain : ne fait rien ;
#   3. sinon mémorise le digest en service (previous), relance le service sur le nouveau digest ;
#   4. contrôle de santé (N essais) : GET /health, puis requête MCP `initialize` si ARR_MCP_TOKEN est fourni ;
#   5. échec : ROLLBACK automatique au digest précédent, journal, code de sortie non nul.
# Codes : 0 ok/à jour, 1 échec + rollback réussi, 2 échec + rollback échoué, 3 échec sans version précédente, 4 erreur d'usage/pull.
# Mode --a-blanc : décrit ce qui serait fait, ne tire rien, ne relance rien.
set -uo pipefail

IMAGE="${IMAGE:-ghcr.io/tojiisamaa/mcp-arr-rust}"
TAG="${TAG:-latest}"                                              # tag suivi ; épingler une version : TAG=1.1.0
COMPOSE_FILE="${COMPOSE_FILE:-/opt/arrmcp/docker-compose.yml}"   # doit contenir : image: ${ARR_IMAGE:-ghcr.io/tojiisamaa/mcp-arr-rust:latest}
SERVICE="${SERVICE:-arrmcp}"
HEALTH_URL="${HEALTH_URL:-http://127.0.0.1:8000/health}"         # /health : {"status":"ok"} sans jeton
MCP_URL="${MCP_URL:-http://127.0.0.1:8000/mcp}"                  # contrôlé seulement si ARR_MCP_TOKEN est dans l'environnement
HEALTH_TRIES="${HEALTH_TRIES:-10}"
HEALTH_WAIT="${HEALTH_WAIT:-3}"
STATE_DIR="${STATE_DIR:-/var/lib/arrmcp-deploy}"
LOG_FILE="${LOG_FILE:-$STATE_DIR/pull-deploy.log}"

A_BLANC=0
case "${1:-}" in
  --a-blanc) A_BLANC=1 ;;
  "") ;;
  *) echo "usage: $0 [--a-blanc]" >&2; exit 4 ;;
esac

log() { local m; m="$(date -u +%FT%TZ) pull-deploy: $*"; echo "$m"; [ "$A_BLANC" = 1 ] || { mkdir -p "$STATE_DIR" 2>/dev/null && echo "$m" >> "$LOG_FILE"; } 2>/dev/null; }

up() { ARR_IMAGE="$1" docker compose -f "$COMPOSE_FILE" up -d "$SERVICE" >/dev/null; }

healthy() {
  local i
  for ((i = 1; i <= HEALTH_TRIES; i++)); do
    if curl -fsS --max-time 5 "$HEALTH_URL" 2>/dev/null | grep -q '"status":"ok"'; then
      if [ -z "${ARR_MCP_TOKEN:-}" ] || curl -fsS --max-time 5 -X POST "$MCP_URL" \
          -H "Authorization: Bearer $ARR_MCP_TOKEN" -H 'Content-Type: application/json' \
          -H 'Accept: application/json, text/event-stream' \
          -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"pull-deploy","version":"1"}}}' 2>/dev/null | grep -q '"result"'; then
        return 0
      fi
    fi
    log "santé non confirmée (essai $i/$HEALTH_TRIES)"
    sleep "$HEALTH_WAIT"
  done
  return 1
}

# Référence en service : état mémorisé, sinon ce que le conteneur utilise (l'id de l'image à défaut de digest).
current_ref() {
  if [ -s "$STATE_DIR/current" ]; then cat "$STATE_DIR/current"; return; fi
  local cid; cid="$(docker compose -f "$COMPOSE_FILE" ps -q "$SERVICE" 2>/dev/null | head -1)"
  [ -n "$cid" ] && docker inspect --format '{{.Image}}' "$cid" 2>/dev/null
  return 0
}

if [ "$A_BLANC" = 1 ]; then
  log "À BLANC : tirerait $IMAGE:$TAG, comparerait son digest à $(cat "$STATE_DIR/current" 2>/dev/null || echo '(aucun état)')"
  log "À BLANC : si différent, relancerait $SERVICE ($COMPOSE_FILE) puis testerait $HEALTH_URL ($HEALTH_TRIES essais)"
  log "À BLANC : en cas d'échec, relancerait le digest précédent (rollback). Rien n'a été exécuté."
  exit 0
fi

mkdir -p "$STATE_DIR" || { echo "STATE_DIR inaccessible" >&2; exit 4; }
exec 9>"$STATE_DIR/lock"
flock -n 9 || { log "un autre déploiement est en cours"; exit 0; }

docker pull "$IMAGE:$TAG" >/dev/null || { log "pull de $IMAGE:$TAG impossible"; exit 4; }
repo_digest="$(docker image inspect --format '{{index .RepoDigests 0}}' "$IMAGE:$TAG" 2>/dev/null)"
case "$repo_digest" in *@sha256:*) ;; *) log "digest de $IMAGE:$TAG illisible"; exit 4 ;; esac
new="$IMAGE@${repo_digest#*@}"
cur="$(current_ref)"

if [ "$new" = "$cur" ] && healthy; then
  log "déjà à jour : $new"
  exit 0
fi

log "déploiement $cur -> $new"
[ -n "$cur" ] && echo "$cur" > "$STATE_DIR/previous"
if up "$new" && healthy; then
  echo "$new" > "$STATE_DIR/current"
  log "OK : $new en service"
  exit 0
fi

log "ÉCHEC de santé sur $new"
prev="$(cat "$STATE_DIR/previous" 2>/dev/null || true)"
if [ -z "$prev" ] || [ "$prev" = "$new" ]; then
  log "aucune version précédente : rollback impossible"
  exit 3
fi
log "ROLLBACK vers $prev"
if up "$prev" && healthy; then
  echo "$prev" > "$STATE_DIR/current"
  log "rollback réussi : $prev en service ($new refusé)"
  exit 1
fi
log "ROLLBACK ÉCHOUÉ : intervention humaine nécessaire"
exit 2
