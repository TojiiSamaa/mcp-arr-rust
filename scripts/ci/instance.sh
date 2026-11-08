#!/usr/bin/env bash
# Niveau instance jetable : Sonarr, Radarr, Lidarr, Prowlarr en conteneurs aux versions ÉPINGLÉES, clé d'API posée par
# variable d'environnement, remplis légèrement, puis les outils de lecture rejoués via arrmcp (stdio) contre eux.
# Prérequis : binaire release (scripts/ci/build-release.sh). Tracearr : seulement si TRACEARR_URL et TRACEARR_API_KEY
# sont fournis (voir CI-A-VERIFIER.md). Aucun secret : la clé est aléatoire, locale, détruite avec les conteneurs.
. "$(dirname "$0")/lib.sh"
NET=arr-ci
KEY="$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')"
declare -A IMG=(
  [sonarr]=lscr.io/linuxserver/sonarr:4.0.20.3014-ls329@sha256:44b06045a98bc1f237b9ffefc4b28dab64cb29305e4a2a94875ae93b5224e4cc
  [radarr]=lscr.io/linuxserver/radarr:6.4.4.10685-ls325@sha256:ade5937813c6ae415b29db729386e53fea3ecf067a019849bafbe2d2a6ec02ba
  [lidarr]=lscr.io/linuxserver/lidarr:3.1.6.5079-ls47@sha256:d5707cfe084da21a3864c7ef0edd9c0543b5e26c79c551f84dad5ef8683af8c7
  [prowlarr]=lscr.io/linuxserver/prowlarr:2.7.0.5723-ls167@sha256:431d7efebcef466e56adaa40a7a2fa2b0e85159a037b8bf465eaadbf0cf5bedf
)
cleanup() { for s in "${!IMG[@]}"; do docker rm -f "ci-$s" >/dev/null 2>&1 || true; done; docker network rm $NET >/dev/null 2>&1 || true; }
trap cleanup EXIT; cleanup
docker network create $NET >/dev/null
for s in "${!IMG[@]}"; do
  U=$(echo "$s" | tr a-z A-Z)
  docker run -d --name "ci-$s" --network $NET --tmpfs /media:mode=1777 \
    -e PUID=1000 -e PGID=1000 -e "${U}__AUTH__APIKEY=$KEY" "${IMG[$s]}" >/dev/null
done

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
docker run --rm -i --network $NET -v $BIN_DIR_VOL:/target -e KEY="$KEY" -e CALLS="$CALLS" \
  -e TRACEARR_URL="${TRACEARR_URL:-}" -e TRACEARR_API_KEY="${TRACEARR_API_KEY:-}" \
  "$RUST_IMAGE" sh -s <<'INNER'
set -eu
apk add -q --no-cache jq >/dev/null
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
[ -n "$TRACEARR_URL" ] && export TRACEARR_URL TRACEARR_API_KEY || { unset TRACEARR_URL TRACEARR_API_KEY; echo "tracearr: non démarré (voir CI-A-VERIFIER.md)"; }

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
[ ! -s /tmp/failed ] || exit 1
INNER
echo "instance: OK"
