#!/usr/bin/env bash
# Niveau instance jetable : Sonarr, Radarr, Lidarr, Prowlarr et Tracearr en conteneurs aux versions ÉPINGLÉES, clés
# d'API aléatoires (*arr : variable d'environnement ; Tracearr : compte propriétaire puis clé `trr_pub_` créés par son
# API), remplis légèrement, puis rejoués via arrmcp (stdio) : lectures, puis écritures (instance-writes.sh).
# Prérequis : binaire release (scripts/ci/build-release.sh). Aucun secret : tout est aléatoire, local, détruit avec
# les conteneurs. NO_TRACEARR=1 saute Tracearr (3 Go de RAM).
. "$(dirname "$0")/lib.sh"
NET=arr-ci
KEY="$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')"
declare -A IMG=(
  [sonarr]=lscr.io/linuxserver/sonarr:4.0.20.3014-ls329@sha256:44b06045a98bc1f237b9ffefc4b28dab64cb29305e4a2a94875ae93b5224e4cc
  [radarr]=lscr.io/linuxserver/radarr:6.4.4.10685-ls325@sha256:ade5937813c6ae415b29db729386e53fea3ecf067a019849bafbe2d2a6ec02ba
  [lidarr]=lscr.io/linuxserver/lidarr:3.1.6.5079-ls47@sha256:d5707cfe084da21a3864c7ef0edd9c0543b5e26c79c551f84dad5ef8683af8c7
  [prowlarr]=lscr.io/linuxserver/prowlarr:2.7.0.5723-ls167@sha256:431d7efebcef466e56adaa40a7a2fa2b0e85159a037b8bf465eaadbf0cf5bedf
)
# Image « supervised » : Tracearr + TimescaleDB + Redis dans un conteneur, secrets générés au premier démarrage.
TRACEARR_IMG=ghcr.io/connorgallopo/tracearr:supervised-3.1.7@sha256:d97688777ad3800d672766608f243685c8c4d6c52cecfc0b6c826563d7aab5eb
cleanup() { for s in "${!IMG[@]}" tracearr; do docker rm -f "ci-$s" >/dev/null 2>&1 || true; done; docker network rm $NET >/dev/null 2>&1 || true; }
trap cleanup EXIT; cleanup
docker network create $NET >/dev/null
for s in "${!IMG[@]}"; do
  U=$(echo "$s" | tr a-z A-Z)
  docker run -d --name "ci-$s" --network $NET --tmpfs /media:mode=1777 \
    -e PUID=1000 -e PGID=1000 -e "${U}__AUTH__APIKEY=$KEY" "${IMG[$s]}" >/dev/null
done
TR=""
if [ -z "${NO_TRACEARR:-}" ]; then
  docker run -d --name ci-tracearr --network $NET --memory 3g --shm-size 512m --ulimit nofile=65536:65536 "$TRACEARR_IMG" >/dev/null
  i=0; until docker exec ci-tracearr curl -sf http://127.0.0.1:3000/health 2>/dev/null | grep -q '"mode":"ready"'; do
    i=$((i+1)); [ $i -le 100 ] || { echo "ECHEC: tracearr ne répond pas après 300 s"; exit 1; }; sleep 3
  done
  # Premier compte = propriétaire (API d'inscription), puis sa clé d'API publique `trr_pub_`.
  tra() { docker exec ci-tracearr curl -sf -H 'Origin: http://127.0.0.1:3000' -H 'Content-Type: application/json' "$@"; }
  PW="ci-$KEY"; T=http://127.0.0.1:3000/api/v1
  tra -d "{\"email\":\"ci@example.invalid\",\"password\":\"$PW\",\"name\":\"ci\",\"username\":\"ciowner\"}" $T/auth/sign-up/email >/dev/null
  TOK=$(tra -d "{\"email\":\"ci@example.invalid\",\"password\":\"$PW\"}" $T/auth/sign-in/email | jq -r .token)
  TRACEARR_API_KEY=$(tra -X POST -H "Authorization: Bearer $TOK" -d '{}' $T/settings/api-key/regenerate | jq -r .token)
  case "$TRACEARR_API_KEY" in trr_pub_*) echo "up: tracearr (propriétaire et clé publique créés)" ;; *) echo "ECHEC: clé Tracearr non créée"; exit 1 ;; esac
  # Données FICTIVES (aucune vraie lecture possible sans serveur de médias) : un serveur, un membre, 5 lectures terminées.
  docker exec -i ci-tracearr psql -q -U tracearr -d tracearr -v ON_ERROR_STOP=1 >/dev/null <<'SQL'
INSERT INTO servers (id, name, type, url, token) VALUES ('11111111-1111-4111-8111-111111111111', 'srv-fictif', 'jellyfin', 'http://jellyfin-fictif.invalid:8096', 'jeton-fictif');
INSERT INTO server_users (id, user_id, server_id, external_id, username) SELECT '22222222-2222-4222-8222-000000000001', id, '11111111-1111-4111-8111-111111111111', 'ext-1', 'membre-fictif' FROM users LIMIT 1;
INSERT INTO sessions (id, server_id, server_user_id, session_key, state, media_type, media_title, ip_address, started_at, stopped_at, last_seen_at, duration_ms, total_duration_ms, progress_ms, watched)
  SELECT ('33333333-3333-4333-8333-00000000000' || i)::uuid, '11111111-1111-4111-8111-111111111111', '22222222-2222-4222-8222-000000000001', 'k' || i, 'stopped', 'movie', 'Film fictif ' || i, '192.0.2.' || i,
         now() - (i || ' hours')::interval, now() - (i || ' hours')::interval + interval '30 min', now(), 1800000, 3600000, 1800000, false
  FROM generate_series(1, 5) i;
SQL
  TR=http://ci-tracearr:3000
fi
export TRACEARR_API_KEY="${TRACEARR_API_KEY:-}"

# Appels rejoués : « outil|arguments JSON ». Préfixe « ? » = non bloquant (dépend d'Internet : recherche de métadonnées).
CALLS='arr_status|{}
arr_list|{"service":"sonarr","resource":"series"}
arr_list|{"service":"sonarr","resource":"tag"}
arr_list|{"service":"sonarr","resource":"rootfolder"}
arr_list|{"service":"sonarr","resource":"qualityprofile"}
arr_list|{"service":"radarr","resource":"movie"}
arr_list|{"service":"radarr","resource":"tag"}
arr_list|{"service":"radarr","resource":"rootfolder"}
arr_list|{"service":"lidarr","resource":"artist"}
arr_list|{"service":"lidarr","resource":"tag"}
arr_list|{"service":"prowlarr","resource":"indexer"}
arr_list|{"service":"prowlarr","resource":"tag"}
arr_get|{"service":"sonarr","resource":"tag","id":1}
arr_get|{"service":"sonarr","resource":"tag","id":999999}
arr_queue|{"service":"sonarr"}
arr_queue|{"service":"radarr"}
arr_history|{"service":"sonarr"}
arr_history|{"service":"radarr"}
arr_wanted|{"service":"sonarr","kind":"missing"}
arr_wanted|{"service":"radarr","kind":"missing"}
arr_calendar|{"service":"sonarr"}
arr_calendar|{"service":"radarr"}
arr_command_status|{"service":"sonarr"}
arr_command_status|{"service":"radarr"}
arr_search_indexers|{"service":"prowlarr","query":"ci"}
arr_api_get|{"service":"sonarr","path":"/system/status"}
arr_api_get|{"service":"prowlarr","path":"/health"}
?arr_lookup|{"service":"sonarr","term":"test"}
?arr_lookup|{"service":"radarr","term":"test"}
arr_stats|{}'

BIN_DIR_VOL=arr-target
docker run --rm -i --network $NET -v $BIN_DIR_VOL:/target -v "$REPO/scripts/ci":/ci:ro -e KEY="$KEY" -e CALLS="$CALLS" -e TR="$TR" -e TRACEARR_API_KEY \
  "$RUST_IMAGE" sh -s <<'INNER'
set -eu
apk add -q --no-cache jq curl >/dev/null
api() { # api <service> <port> <version> <method> <path> [json]
  if [ "$4" = GET ]; then wget -qO- --header "X-Api-Key: $KEY" "http://ci-$1:$2/api/$3$5"
  else wget -qO- --header "X-Api-Key: $KEY" --header 'Content-Type: application/json' --post-data "$6" "http://ci-$1:$2/api/$3$5"; fi
}
declare_svc() { echo "sonarr:8989:v3 radarr:7878:v3 lidarr:8686:v1 prowlarr:9696:v1"; }
for e in $(declare_svc); do
  s=${e%%:*}; r=${e#*:}; p=${r%%:*}; v=${r#*:}
  i=0; until api "$s" "$p" "$v" GET /system/status >/dev/null 2>&1; do
    i=$((i+1)); [ $i -le 90 ] || { echo "ECHEC: $s ne répond pas après 180 s"; exit 1; }; sleep 2
  done
  echo "up: $s ($(api "$s" "$p" "$v" GET /system/status | jq -r .version))"
  api "$s" "$p" "$v" POST /tag '{"label":"ci"}' >/dev/null
done
api sonarr 8989 v3 POST /rootfolder '{"path":"/media"}' >/dev/null || echo "WARN: rootfolder sonarr non créé"
api radarr 7878 v3 POST /rootfolder '{"path":"/media"}' >/dev/null || echo "WARN: rootfolder radarr non créé"

export SONARR_URL=http://ci-sonarr:8989 SONARR_API_KEY=$KEY RADARR_URL=http://ci-radarr:7878 RADARR_API_KEY=$KEY \
       LIDARR_URL=http://ci-lidarr:8686 LIDARR_API_KEY=$KEY PROWLARR_URL=http://ci-prowlarr:9696 PROWLARR_API_KEY=$KEY
[ -n "$TR" ] && export TRACEARR_URL=$TR || { unset TRACEARR_URL TRACEARR_API_KEY; echo "tracearr: sauté (NO_TRACEARR)"; }

{
  echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"ci","version":"0"}}}'
  echo '{"jsonrpc":"2.0","method":"notifications/initialized"}'
  n=10; echo "$CALLS" | while IFS='|' read -r t a; do
    n=$((n+1)); echo "{\"jsonrpc\":\"2.0\",\"id\":$n,\"method\":\"tools/call\",\"params\":{\"name\":\"${t#\?}\",\"arguments\":$a}}"
  done
} > /tmp/req.jsonl
/target/release/arrmcp --transport stdio < /tmp/req.jsonl > /tmp/resp.jsonl
fail=0; n=10
echo "$CALLS" | while IFS='|' read -r t a; do
  n=$((n+1))
  r=$(jq -c "select(.id==$n)" /tmp/resp.jsonl)
  if [ -n "$r" ] && [ "$(echo "$r" | jq '(.error==null) and ((.result.isError // false)==false)')" = true ]; then echo "ok   ${t#\?} $a"
  elif [ "${t#\?}" != "$t" ]; then echo "warn ${t#\?} $a"
  else echo "FAIL $t $a -> $(echo "$r" | head -c 300)"; echo x >> /tmp/failed; fi
done
. /ci/instance-writes.sh
[ ! -s /tmp/failed ] || { echo "ECHECS : $(wc -l < /tmp/failed)"; exit 1; }
INNER
echo "instance: OK"
