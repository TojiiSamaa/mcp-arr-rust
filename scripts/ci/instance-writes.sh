set +e  # les échecs sont comptés dans /tmp/failed, pas par sortie immédiate
# Niveau « écritures » du test d'instance jetable (sourcé DANS le conteneur par instance.sh, après les lectures).
# Entrées : KEY, SONARR_URL… PROWLARR_URL (+ *_API_KEY), TRACEARR_URL/TRACEARR_API_KEY facultatifs, binaire /target/release/arrmcp.
# Chaque contrôle compare la réponse d'arrmcp à l'état LU directement sur l'instance (jamais à la seule réponse de l'outil).
# Préfixe « warn » : dépend d'Internet côté instance (recherche de métadonnées), non bloquant.
BIN=/target/release/arrmcp
INIT='{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"ci","version":"0"}}}'
# call <outil> <arguments> : un processus par appel (aucune course entre écritures), affiche le texte du résultat.
call() {
  printf '%s\n' "$INIT" '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
    "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"$1\",\"arguments\":$2}}" |
    timeout 120 "$BIN" --transport stdio 2>/dev/null | jq -c 'select(.id==2) | (.result.content[0].text | fromjson? // .) // {rpc_error: .error.message}'
}
# chk <libellé> <expression jq> <json> : OK si l'expression vaut true sur le json.
chk() {
  if [ "$(printf '%s' "$3" | jq "$2" 2>/dev/null)" = true ]; then echo "ok   $1"
  else echo "FAIL $1 -> $(printf '%s' "$3" | head -c 400)"; echo x >> /tmp/failed; fi
}
# api <url-de-base> <méthode> <chemin> [json] : appel direct à l'instance (contrôle indépendant d'arrmcp).
api() { curl -s -X "$2" -H "X-Api-Key: $KEY" -H 'Content-Type: application/json' ${4:+-d "$4"} "$1$3"; }
code() { curl -s -o /dev/null -w '%{http_code}' -H "X-Api-Key: $KEY" "$1$2"; }
idle() { i=0; while [ $i -lt 60 ]; do # attend que l'instance n'ait plus de commande en cours
  [ "$(api "$1" GET /command | jq '[.[]|select(.status=="queued" or .status=="started")]|length')" = 0 ] && return 0
  i=$((i+1)); sleep 2; done; }
SO=$SONARR_URL/api/v3 RA=$RADARR_URL/api/v3 LI=$LIDARR_URL/api/v1 PR=$PROWLARR_URL/api/v1
SVCS="sonarr:$SO radarr:$RA lidarr:$LI prowlarr:$PR"

echo "--- masquage (lecture, interrupteurs fermés) : RETEST D1/D2"
api "$SO" POST '/notification?forceSave=true' '{"name":"ci-discord","implementation":"Discord","configContract":"DiscordSettings","fields":[{"name":"webHookUrl","value":"https://discord.example/api/webhooks/1/SECRET-WEBHOOK-FICTIF"}],"tags":[]}' >/dev/null
RAW=$(api "$SO" GET /notification; api "$SO" GET /config/host)
chk "témoin : l'instance renvoie bien les secrets en clair" . "$(printf '%s' "$RAW" | grep -c -e SECRET-WEBHOOK-FICTIF -e "$KEY" | jq '. >= 2')"
OUT=$(call arr_list '{"service":"sonarr","resource":"notification"}'; call arr_api_get '{"service":"sonarr","path":"config/host"}'; call arr_api_get '{"service":"prowlarr","path":"config/host"}')
chk "D1 aucune valeur secrète en clair" . "$(printf '%s' "$OUT" | grep -c -e SECRET-WEBHOOK-FICTIF -e "$KEY" | jq '. == 0')"
chk "D2 masque présent (webHookUrl, apiKey sonarr et prowlarr)" . "$(printf '%s' "$OUT" | grep -o '\[redacted\]' | wc -l | jq '. >= 3')"

echo "--- dry-run forcé (ARR_DRY_RUN=true, tous les interrupteurs ouverts, confirm donné) : zéro écriture côté instance"
snap() { for e in $SVCS; do u=${e#*:}; api "$u" GET /tag | jq -c '[.[]|[.id,.label]]'; done
  api "$SO" GET /series | jq -c '[.[].id]'; api "$RA" GET /movie | jq -c '[.[].id]'; api "$SO" GET /notification | jq -c '[.[].id]'
  for u in $SO $RA $LI; do api "$u" GET /command | jq -c '[.[]|select(.trigger=="manual")]|length'; done; }
BEFORE=$(snap)
dry() { (export ARR_DRY_RUN=true ARR_ALLOW_WRITES=true ARR_ALLOW_DELETE=true ARR_CONFIRM_ARG=true; call "$@"); }
for e in $SVCS; do s=${e%%:*}
  chk "dry-run arr_create $s" '.dry_run==true and .would_send[0].method=="POST"' "$(dry arr_create "{\"service\":\"$s\",\"resource\":\"tag\",\"body\":{\"label\":\"ci-dry\"}}")"
  chk "dry-run arr_update $s" '.dry_run==true and .would_send[0].method=="PUT"' "$(dry arr_update "{\"service\":\"$s\",\"resource\":\"tag\",\"id\":1,\"changes\":{\"label\":\"ci-dry\"}}")"
  chk "dry-run arr_delete $s (confirm:true)" '.dry_run==true and .would_send[0].method=="DELETE" and .exists==true' "$(dry arr_delete "{\"service\":\"$s\",\"resource\":\"tag\",\"id\":1,\"confirm\":true}")"
done
for s in sonarr radarr; do
  chk "dry-run arr_command $s RssSync" '.dry_run==true and .would_send[0].path=="/api/v3/command"' "$(dry arr_command "{\"service\":\"$s\",\"name\":\"RssSync\"}")"
  chk "dry-run arr_queue_remove $s" '.dry_run==true and .would_send[0].method=="DELETE"' "$(dry arr_queue_remove "{\"service\":\"$s\",\"id\":1,\"confirm\":true}")"
done
chk "dry-run arr_add sonarr" '.dry_run==true and .would_send[0].body.addOptions.searchForMissingEpisodes==false' "$(dry arr_add '{"service":"sonarr","item":{"tvdbId":81189,"title":"ci"},"qualityProfileId":1,"rootFolderPath":"/media"}')"
chk "dry-run arr_grab_release sonarr" '.dry_run==true and .would_send[0].path=="/api/v3/release"' "$(dry arr_grab_release '{"service":"sonarr","guid":"ci-guid-fictif","indexerId":1}')"
sleep 3
chk "dry-run : état des 4 instances identique (tags, séries, films, notifications, commandes manuelles)" . "$(jq -n --arg a "$BEFORE" --arg b "$(snap)" '$a == $b')"

echo "--- écritures réelles (ARR_ALLOW_WRITES, ARR_ALLOW_DELETE, ARR_CONFIRM_ARG, ARR_AUDIT_LOG)"
export ARR_ALLOW_WRITES=true ARR_ALLOW_DELETE=true ARR_CONFIRM_ARG=true ARR_AUDIT_LOG=/tmp/audit.log
for e in $SVCS; do s=${e%%:*} u=${e#*:}
  R=$(call arr_create "{\"service\":\"$s\",\"resource\":\"tag\",\"body\":{\"label\":\"ci-w-$s\"}}"); ID=$(printf '%s' "$R" | jq '.created.id // 0')
  chk "arr_create tag $s (instance : label présent)" '.label=="ci-w-'"$s"'"' "$(api "$u" GET "/tag/$ID")"
  R=$(call arr_update "{\"service\":\"$s\",\"resource\":\"tag\",\"id\":$ID,\"changes\":{\"label\":\"ci-w2-$s\"}}")
  chk "arr_update tag $s (previous renvoyé)" '.previous.label=="ci-w-'"$s"'"' "$R"
  chk "arr_update tag $s (instance : label changé)" '.label=="ci-w2-'"$s"'"' "$(api "$u" GET "/tag/$ID")"
  chk "arr_delete $s sans confirm : refus" '.error_type=="confirmation_required"' "$(call arr_delete "{\"service\":\"$s\",\"resource\":\"tag\",\"id\":$ID}")"
  chk "arr_delete $s sans confirm (instance : tag toujours là)" '.id=='"$ID" "$(api "$u" GET "/tag/$ID")"
  chk "arr_delete $s avec confirm" '.deleted.id=='"$ID" "$(call arr_delete "{\"service\":\"$s\",\"resource\":\"tag\",\"id\":$ID,\"confirm\":true}")"
  chk "arr_delete $s (instance : 404)" '. == 404' "$(code "$u" "/tag/$ID")"
  chk "arr_get $s après suppression : found:false" '.found==false' "$(call arr_get "{\"service\":\"$s\",\"resource\":\"tag\",\"id\":$ID}")"
done
for c in "sonarr:RefreshSeries:$SO" "radarr:RefreshMovie:$RA" "lidarr:RefreshArtist:$LI"; do s=${c%%:*} r=${c#*:}; n=${r%%:*} u=${r#*:}
  R=$(call arr_command "{\"service\":\"$s\",\"name\":\"$n\"}"); CID=$(printf '%s' "$R" | jq '.command.id // 0')
  chk "arr_command $s $n (instance : commande connue)" '.name=="'"$n"'"' "$(api "$u" GET "/command/$CID")"
  chk "arr_command_status $s $CID" '(.result.status // .status // "") | test("queued|started|completed")' "$(call arr_command_status "{\"service\":\"$s\",\"id\":$CID}")"
done
chk "arr_queue_remove sans confirm : refus" '.error_type=="confirmation_required"' "$(call arr_queue_remove '{"service":"sonarr","id":999999}')"
chk "arr_queue_remove avec confirm, élément absent : erreur typée" '.error_type=="not_found"' "$(call arr_queue_remove '{"service":"sonarr","id":999999,"confirm":true}')"
chk "politique : ARR_WRITE_SERVICES=radarr refuse sonarr" '.error_type=="service_not_allowed"' "$(ARR_WRITE_SERVICES=radarr call arr_command '{"service":"sonarr","name":"RssSync"}')"
chk "politique : commande hors liste blanche" '.error_type=="invalid_input"' "$(call arr_command '{"service":"sonarr","name":"DeleteLogFiles"}')"
chk "D4 : masque renvoyé au service refusé" '.error_type=="invalid_input"' "$(call arr_update '{"service":"sonarr","resource":"tag","id":1,"changes":{"label":"[redacted]"}}')"

# arr_add : JAMAIS de recherche sans `search: true`. Témoin : avec `search: true`, la même détection voit la recherche.
searches() { api "$1" GET /command | jq -c '[.[]|select(.name|test("Search"))|.name]'; }
for a in "sonarr tvdb:81189 $SO series" "radarr tmdb:603 $RA movie"; do
  set -- $a; s=$1 t=$2 u=$3 res=$4
  ITEM=$(call arr_lookup "{\"service\":\"$s\",\"term\":\"$t\",\"limit\":1}" | jq -c '.result.items[0] // empty')
  if [ -z "$ITEM" ]; then echo "warn arr_add $s : lookup impossible (Internet côté instance ?), étape non testée"; continue; fi
  QP=$(api "$u" GET /qualityprofile | jq '.[0].id')
  idle "$u"; S0=$(searches "$u")
  R=$(call arr_add "{\"service\":\"$s\",\"item\":$ITEM,\"qualityProfileId\":$QP,\"rootFolderPath\":\"/media\"}"); ID=$(printf '%s' "$R" | jq '.added.id // 0')
  chk "arr_add $s : ajouté, search_triggered:false" '.added.id > 0 and .search_triggered==false' "$R"
  chk "arr_add $s (instance : objet créé)" '.id=='"$ID" "$(api "$u" GET "/$res/$ID")"
  sleep 3; idle "$u"
  chk "arr_add $s : AUCUNE recherche partie côté instance" . "$(jq -n --arg a "$S0" --arg b "$(searches "$u")" '$a == $b')"
  chk "arr_add $s : file vide" '.totalRecords==0' "$(api "$u" GET /queue)"
  call arr_update "{\"service\":\"$s\",\"resource\":\"$res\",\"id\":$ID,\"changes\":{\"monitored\":false}}" >/dev/null
  chk "arr_update $s $res monitored:false (instance)" '.monitored==false' "$(api "$u" GET "/$res/$ID")"
  chk "arr_delete $s $res avec confirm" '.deleted.id=='"$ID" "$(call arr_delete "{\"service\":\"$s\",\"resource\":\"$res\",\"id\":$ID,\"confirm\":true}")"
  chk "arr_delete $s $res (instance : 404)" '. == 404' "$(code "$u" "/$res/$ID")"
  if [ "$s" = sonarr ]; then # témoin positif de la détection
    R=$(call arr_add "{\"service\":\"$s\",\"item\":$ITEM,\"qualityProfileId\":$QP,\"rootFolderPath\":\"/media\",\"search\":true}"); ID=$(printf '%s' "$R" | jq '.added.id // 0')
    sleep 3; idle "$u"
    chk "témoin : avec search:true la détection voit une recherche" . "$(jq -n --arg a "$S0" --arg b "$(searches "$u")" '$a != $b')"
    call arr_delete "{\"service\":\"$s\",\"resource\":\"$res\",\"id\":$ID,\"confirm\":true}" >/dev/null
  fi
done
chk "audit.log : une ligne par écriture, sans clé" . "$(jq -n --argjson n "$(wc -l < /tmp/audit.log)" --argjson k "$(grep -c -e "$KEY" -e trr_pub_ /tmp/audit.log || true)" '$n >= 20 and $k == 0')"
unset ARR_ALLOW_WRITES ARR_ALLOW_DELETE ARR_CONFIRM_ARG ARR_AUDIT_LOG

[ -n "${TRACEARR_URL:-}" ] || { echo "warn tracearr non configuré : bloc G non testé"; return 0; }
echo "--- Tracearr (RETEST G) : contrôle direct de l'API puis arrmcp"
TB="Authorization: Bearer $TRACEARR_API_KEY"
tcode() { curl -s -o /dev/null -w '%{http_code}' -H "$TB" "$TRACEARR_URL$1"; }
chk "G0 v2 streams et v1 stats en direct : 200" '. == "200 200"' "\"$(tcode /api/v2/public/streams) $(tcode /api/v1/public/stats)\""
chk "G1 arr_status : tracearr ok" '[.services[]|select(.service=="tracearr" and .ok==true)]|length==1' "$(call arr_status '{}')"
for r in streams history violations servers users libraries recently-added stats "watched-media:movie" "top-media:movie"; do
  f=${r#*:}; r=${r%%:*}; [ "$f" = "$r" ] && f='{}' || f="{\"media_type\":\"$f\"}"
  chk "G2 arr_list tracearr $r" '.error_type==null and (.result|type)=="object"' "$(call arr_list "{\"service\":\"tracearr\",\"resource\":\"$r\",\"pageSize\":5,\"filters\":$f}")"
done
chk "G2 watched-media sans media_type : refus avant requête" '.error_type=="invalid_input"' "$(call arr_list '{"service":"tracearr","resource":"watched-media"}')"
# G3 : 5 lectures fictives semées par instance.sh, pages de 2 par curseur.
P=$(call arr_list '{"service":"tracearr","resource":"history","pageSize":2}'); IDS=$(printf '%s' "$P" | jq -c '[.result.data[].id]'); N=0
while C=$(printf '%s' "$P" | jq -r '.result.meta.nextCursor // empty'); [ -n "$C" ] && [ $N -lt 5 ]; do
  P=$(call arr_list "{\"service\":\"tracearr\",\"resource\":\"history\",\"pageSize\":2,\"cursor\":\"$C\"}"); IDS=$(jq -nc --argjson a "$IDS" --argjson b "$(printf '%s' "$P" | jq -c '[.result.data[].id]')" '$a+$b'); N=$((N+1)); done
chk "G3 curseur : 3 pages, 5 lectures distinctes, dernière page sans curseur" '(length==5) and (unique|length==5)' "$IDS"
chk "G4 media inconnu : found:false" '.found==false' "$(call arr_api_get '{"service":"tracearr","path":"media/ref-inconnue"}')"
chk "G5 ressource hors liste blanche" '.error_type=="resource_not_allowed"' "$(call arr_list '{"service":"tracearr","resource":"settings"}')"
SID=00000000-0000-4000-8000-000000000000
chk "G6a interrupteurs fermés : outil absent" '.rpc_error|test("Unknown tool")' "$(call arr_terminate_stream "{\"id\":\"$SID\"}")"
chk "G6b dry-run : POST /api/v1/public/streams/{id}/terminate" '.dry_run==true and .would_send[0].path=="/api/v1/public/streams/'$SID'/terminate"' "$(dry arr_terminate_stream "{\"id\":\"$SID\",\"confirm\":true}")"
export ARR_ALLOW_WRITES=true ARR_ALLOW_DELETE=true ARR_AUDIT_LOG=/tmp/audit.log
chk "G6c sans confirmation : refus" '.error_type=="confirmation_required"' "$(call arr_terminate_stream "{\"id\":\"$SID\"}")"
export ARR_CONFIRM_ARG=true
chk "G6d avec confirmation, session inconnue : not_found (le service a reçu l'appel)" '.error_type=="not_found" and .http_status==404' "$(call arr_terminate_stream "{\"id\":\"$SID\",\"confirm\":true}")"
chk "G6e id non UUID : refus avant requête" '.error_type=="invalid_input"' "$(call arr_terminate_stream '{"id":"abc","confirm":true}')"
unset ARR_ALLOW_WRITES ARR_ALLOW_DELETE ARR_CONFIRM_ARG ARR_AUDIT_LOG
chk "aucune clé trr_pub_ dans audit.log" . "$(grep -c "$TRACEARR_API_KEY" /tmp/audit.log | jq '. == 0')"
