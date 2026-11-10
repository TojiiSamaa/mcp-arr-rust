//! Bout en bout contre un faux serveur *arr/Tracearr qui ENREGISTRE chaque requête : les tests prouvent ce que le
//! serveur MCP envoie réellement (comptage côté service), pas ce qu'il prétend. Cinq preuves + les deux transports.
mod common;
use common::*;
use serde_json::{json, Value};

const WRITES: (&str, &str) = ("ARR_ALLOW_WRITES", "true");
const DELETE: (&str, &str) = ("ARR_ALLOW_DELETE", "true");

/// Un appel valide par outil d'écriture ou destructif, `dry_run:false` explicite (le serveur doit passer outre).
fn write_calls() -> Vec<(&'static str, Value)> {
    vec![
        (
            "arr_add",
            json!({ "service": "sonarr", "item": { "title": "T", "tvdbId": 1 }, "rootFolderPath": "/tv", "qualityProfileId": 1, "dry_run": false }),
        ),
        (
            "arr_update",
            json!({ "service": "sonarr", "resource": "series", "id": 1, "changes": { "monitored": false }, "dry_run": false }),
        ),
        (
            "arr_command",
            json!({ "service": "sonarr", "name": "RefreshSeries", "dry_run": false }),
        ),
        (
            "arr_grab_release",
            json!({ "service": "radarr", "guid": "g1", "indexerId": 1, "dry_run": false }),
        ),
        (
            "arr_create",
            json!({ "service": "sonarr", "resource": "tag", "body": { "label": "x" }, "dry_run": false }),
        ),
        (
            "arr_test",
            json!({ "service": "prowlarr", "resource": "indexer", "body": { "name": "i" }, "dry_run": false }),
        ),
        (
            "arr_delete",
            json!({ "service": "sonarr", "resource": "series", "id": 1, "delete_files": true, "dry_run": false }),
        ),
        (
            "arr_queue_remove",
            json!({ "service": "radarr", "id": 1, "remove_from_client": true, "blocklist": true, "dry_run": false }),
        ),
        (
            "arr_terminate_stream",
            json!({ "id": "00000000-0000-4000-8000-000000000001", "dry_run": false }),
        ),
    ]
}

// ---- Preuve 1 : défauts fermés ----

#[tokio::test]
async fn closed_by_default_no_write_tool_is_listed_or_callable() {
    let (s, log) = start(&[]).await;
    assert!(
        write_tools(&s).await.is_empty(),
        "outils d'écriture exposés par défaut : {:?}",
        write_tools(&s).await
    );
    assert_eq!(
        names(&s).await.len(),
        13,
        "l'état fermé doit montrer les 13 outils de lecture : {:?}",
        names(&s).await
    );
    for (tool, args) in write_calls() {
        let r = rpc(&s, "tools/call", json!({ "name": tool, "arguments": args })).await;
        let refused = r.get("error").is_some() || r["result"]["isError"] == true;
        assert!(refused, "{tool} accepté sans ARR_ALLOW_WRITES : {r}");
    }
    assert_eq!(writes_received(&log), 0, "{:?}", log.lock().unwrap());
}

#[tokio::test]
async fn writes_alone_do_not_expose_destructive_tools() {
    let (s, log) = start(&[WRITES]).await;
    let n = names(&s).await;
    assert!(
        n.contains(&"arr_add".into()) && n.contains(&"arr_command".into()),
        "{n:?}"
    );
    for t in ["arr_delete", "arr_queue_remove", "arr_terminate_stream"] {
        assert!(!n.contains(&t.into()), "{t} exposé sans ARR_ALLOW_DELETE");
        let r = rpc(
            &s,
            "tools/call",
            json!({ "name": t, "arguments": { "service": "sonarr", "id": 1 } }),
        )
        .await;
        assert!(
            r.get("error").is_some() || r["result"]["isError"] == true,
            "{t}: {r}"
        );
    }
    assert!(sent(&log, "DELETE").is_empty() && sent(&log, "POST").is_empty());
}

// ---- Preuve 2 : dry-run forcé ----

#[tokio::test]
async fn dry_run_env_forces_the_simulation_for_every_write_and_destructive_tool() {
    let (s, log) = start(&[
        WRITES,
        DELETE,
        ("ARR_ELICIT", "off"),
        ("ARR_DRY_RUN", "true"),
    ])
    .await;
    // La table couvre exactement les outils non lecture publiés : un outil ajouté sans test fait échouer ici.
    let mut listed = write_tools(&s).await;
    let mut covered: Vec<String> = write_calls().iter().map(|(n, _)| n.to_string()).collect();
    listed.sort();
    covered.sort();
    assert_eq!(
        listed, covered,
        "la table d'appels doit couvrir tous les outils d'écriture publiés"
    );
    for (tool, args) in write_calls() {
        let (err, v) = call(&s, tool, args).await;
        assert!(!err, "{tool}: {v}");
        assert_eq!(v["dry_run"], true, "{tool}: {v}");
    }
    assert_eq!(
        writes_received(&log),
        0,
        "le service a reçu une écriture : {:?}",
        log.lock().unwrap()
    );
}

#[tokio::test]
async fn dry_run_alone_lists_and_previews_every_write_but_sends_none() {
    // Comportement du modèle netbox, figé : un dry-run n'envoie rien, donc ARR_DRY_RUN=true seul suffit à lister
    // aussi les outils destructifs (sans ARR_ALLOW_WRITES ni ARR_ALLOW_DELETE).
    let (s, log) = start(&[("ARR_DRY_RUN", "true")]).await;
    let mut listed = write_tools(&s).await;
    let mut covered: Vec<String> = write_calls().iter().map(|(n, _)| n.to_string()).collect();
    listed.sort();
    covered.sort();
    assert_eq!(listed, covered);
    assert_eq!(names(&s).await.len(), 22);
    for (tool, args) in write_calls() {
        let (err, v) = call(&s, tool, args).await;
        assert!(!err && v["dry_run"] == true, "{tool}: {v}");
    }
    assert_eq!(writes_received(&log), 0, "{:?}", log.lock().unwrap());
}

// ---- Preuve 3 : refus avant requête (le service ne reçoit RIEN) ----

#[tokio::test]
async fn delete_switch_refuses_before_any_request() {
    let (s, log) = start(&[WRITES]).await;
    let r = rpc(&s, "tools/call", json!({ "name": "arr_delete", "arguments": { "service": "sonarr", "resource": "series", "id": 1 } })).await;
    assert!(
        r.get("error").is_some() || r["result"]["isError"] == true,
        "{r}"
    );
    assert_eq!(received(&log), 0);
}

#[tokio::test]
async fn service_allowlist_refuses_other_services_before_any_request() {
    let (s, log) = start(&[WRITES, ("ARR_WRITE_SERVICES", "sonarr")]).await;
    let (err, v) = call(
        &s,
        "arr_command",
        json!({ "service": "radarr", "name": "RefreshMovie" }),
    )
    .await;
    assert!(err, "{v}");
    assert_eq!(received(&log), 0, "{:?}", log.lock().unwrap());
    // le service autorisé passe
    let (err, v) = call(
        &s,
        "arr_command",
        json!({ "service": "sonarr", "name": "RefreshSeries" }),
    )
    .await;
    assert!(!err, "{v}");
    assert_eq!(sent(&log, "POST").len(), 1);
}

#[tokio::test]
async fn command_outside_the_allowlist_never_reaches_the_service() {
    let (s, log) = start(&[WRITES]).await;
    let (err, v) = call(
        &s,
        "arr_command",
        json!({ "service": "sonarr", "name": "DeleteEverything" }),
    )
    .await;
    assert!(err, "{v}");
    assert_eq!(received(&log), 0);
}

#[tokio::test]
async fn destructive_calls_fail_closed_without_a_way_to_confirm() {
    // Défaut (ARR_ELICIT=destructive) et session sans canal : refus typé avant toute requête.
    let (s, log) = start(&[WRITES, DELETE]).await;
    for (tool, args) in write_calls()
        .into_iter()
        .filter(|(n, _)| ["arr_delete", "arr_queue_remove", "arr_terminate_stream"].contains(n))
    {
        let (err, v) = call(&s, tool, args).await;
        assert!(
            err && v.to_string().contains("confirmation_required"),
            "{tool}: {v}"
        );
    }
    assert_eq!(received(&log), 0, "{:?}", log.lock().unwrap());
}

#[tokio::test]
async fn confirm_argument_is_an_opt_in_fallback() {
    let del = json!({ "service": "sonarr", "resource": "series", "id": 1 });
    // Sans l'option, `confirm: true` ne change rien.
    let (s, log) = start(&[WRITES, DELETE]).await;
    let (err, v) = call(
        &s,
        "arr_delete",
        json!({ "service": "sonarr", "resource": "series", "id": 1, "confirm": true }),
    )
    .await;
    assert!(
        err && v.to_string().contains("confirmation_required"),
        "{v}"
    );
    assert_eq!(received(&log), 0);
    // Avec ARR_CONFIRM_ARG : refus sans l'argument, succès avec.
    let (s, log) = start(&[WRITES, DELETE, ("ARR_CONFIRM_ARG", "true")]).await;
    let (err, v) = call(&s, "arr_delete", del.clone()).await;
    assert!(err && v.to_string().contains("confirm"), "{v}");
    assert!(sent(&log, "DELETE").is_empty());
    let mut ok = del;
    ok["confirm"] = json!(true);
    let (err, v) = call(&s, "arr_delete", ok).await;
    assert!(!err, "{v}");
    assert_eq!(sent(&log, "DELETE").len(), 1);
}

#[tokio::test]
async fn elicitation_decline_sends_nothing_and_accept_sends_the_delete() {
    let (s, log, mut rx) = start_with(&[WRITES, DELETE], true).await;
    rpc(&s, "initialize", json!({ "protocolVersion": "2025-06-18", "capabilities": { "elicitation": {} }, "clientInfo": { "name": "t", "version": "1" } })).await;
    for (accept, deletes) in [(false, 0), (true, 1)] {
        let s2 = s.clone();
        let task = tokio::spawn(async move {
            call(
                &s2,
                "arr_delete",
                json!({ "service": "sonarr", "resource": "series", "id": 1 }),
            )
            .await
        });
        let req = rx.recv().await.unwrap();
        assert_eq!(req["method"], "elicitation/create");
        let result = if accept {
            json!({ "action": "accept", "content": { "confirm": true } })
        } else {
            json!({ "action": "decline" })
        };
        s.deliver_response(json!({ "jsonrpc": "2.0", "id": req["id"], "result": result }));
        let (err, v) = task.await.unwrap();
        assert_eq!(err, !accept, "{v}");
        assert_eq!(sent(&log, "DELETE").len(), deletes);
    }
}

// ---- Preuve 4 : contrat ----

#[tokio::test]
async fn every_published_schema_is_closed_and_confirm_follows_the_option() {
    for confirm_arg in [false, true] {
        let mut env = vec![WRITES, DELETE];
        if confirm_arg {
            env.push(("ARR_CONFIRM_ARG", "true"));
        }
        let (s, _) = start(&env).await;
        for t in tools(&s).await {
            let name = t["name"].as_str().unwrap();
            assert!(name.starts_with("arr_"), "{name}");
            assert_eq!(t["inputSchema"]["additionalProperties"], false, "{name}");
            assert_eq!(t["inputSchema"]["type"], "object", "{name}");
            let destructive = t["annotations"]["destructiveHint"] == true;
            assert_eq!(
                t["inputSchema"]["properties"].get("confirm").is_some(),
                destructive && confirm_arg,
                "{name} confirm_arg={confirm_arg}"
            );
        }
    }
}

#[tokio::test]
async fn the_published_tool_list_matches_the_contract() {
    let (s, _) = start(&[WRITES, DELETE]).await;
    let n = names(&s).await;
    for t in [
        "arr_status",
        "arr_list",
        "arr_get",
        "arr_lookup",
        "arr_queue",
        "arr_history",
        "arr_wanted",
        "arr_calendar",
        "arr_releases",
        "arr_command_status",
        "arr_search_indexers",
        "arr_api_get",
        "arr_stats",
        "arr_add",
        "arr_update",
        "arr_command",
        "arr_grab_release",
        "arr_create",
        "arr_test",
        "arr_delete",
        "arr_queue_remove",
        "arr_terminate_stream",
    ] {
        assert!(n.contains(&t.to_string()), "outil du contrat absent : {t}");
    }
    assert_eq!(n.len(), 22, "{n:?}");
}

#[tokio::test]
async fn a_service_without_configuration_is_refused_with_a_stable_error() {
    let (s, log) = start(&[("RADARR_URL", ""), ("RADARR_API_KEY", "")]).await;
    let (err, v) = call(
        &s,
        "arr_list",
        json!({ "service": "radarr", "resource": "movie" }),
    )
    .await;
    assert!(err && v.to_string().contains("not_configured"), "{v}");
    assert_eq!(received(&log), 0);
    let (err, v) = call(&s, "arr_status", json!({})).await;
    assert!(!err, "{v}");
    assert!(
        !v.to_string().contains("radarr"),
        "service non configuré listé : {v}"
    );
}

#[tokio::test]
async fn reading_a_missing_object_is_an_answer_not_a_tool_error() {
    let (s, _) = start(&[]).await;
    let (err, v) = call(
        &s,
        "arr_get",
        json!({ "service": "sonarr", "resource": "series", "id": 404 }),
    )
    .await;
    assert!(!err && v["found"] == false, "{v}");
}

#[tokio::test]
async fn an_html_404_is_an_error_that_does_not_copy_the_page() {
    let (s, _) = start(&[]).await;
    let (err, v) = call(
        &s,
        "arr_api_get",
        json!({ "service": "sonarr", "path": "/nothing" }),
    )
    .await;
    assert!(err, "{v}");
    assert!(!v.to_string().contains("<html>"), "{v}");
}

#[tokio::test]
async fn reads_redact_secrets_and_use_each_services_authentication() {
    let (s, log) = start(&[]).await;
    for (service, resource, key) in [
        ("sonarr", "series", "key-sonarr"),
        ("radarr", "movie", "key-radarr"),
        ("lidarr", "artist", "key-lidarr"),
        ("prowlarr", "indexer", "key-prowlarr"),
    ] {
        let (err, v) = call(
            &s,
            "arr_list",
            json!({ "service": service, "resource": resource }),
        )
        .await;
        assert!(!err, "{service}: {v}");
        assert!(
            !v.to_string().contains(SECRET),
            "{service} : secret non masqué : {v}"
        );
        let r = log.lock().unwrap().last().unwrap().clone();
        assert_eq!(
            (r.api_key.as_deref(), r.bearer),
            (Some(key), None),
            "{service}"
        );
    }
    let (err, v) = call(
        &s,
        "arr_list",
        json!({ "service": "tracearr", "resource": "streams" }),
    )
    .await;
    assert!(!err, "{v}");
    assert!(!v.to_string().contains(SECRET), "{v}");
    let r = log.lock().unwrap().last().unwrap().clone();
    assert_eq!(
        (r.path.as_str(), r.api_key, r.bearer.as_deref()),
        ("/api/v2/public/streams", None, Some("Bearer trr_pub_test"))
    );
}

#[tokio::test]
async fn api_prefixes_follow_the_service() {
    let (s, log) = start(&[]).await;
    for (service, resource, prefix) in [
        ("sonarr", "series", "/api/v3/"),
        ("lidarr", "artist", "/api/v1/"),
        ("prowlarr", "indexer", "/api/v1/"),
    ] {
        call(
            &s,
            "arr_list",
            json!({ "service": service, "resource": resource }),
        )
        .await;
        let p = log.lock().unwrap().last().unwrap().path.clone();
        assert!(p.starts_with(prefix), "{service}: {p}");
    }
}

#[tokio::test]
async fn a_resource_outside_the_allowlist_is_refused_before_the_request() {
    let (s, log) = start(&[]).await;
    let (err, v) = call(
        &s,
        "arr_list",
        json!({ "service": "sonarr", "resource": "system/backup/../../config/host" }),
    )
    .await;
    assert!(err, "{v}");
    assert_eq!(received(&log), 0);
}

// ---- arr_add : jamais de recherche automatique ----

#[tokio::test]
async fn add_never_triggers_a_search_unless_asked() {
    let (s, log) = start(&[WRITES]).await;
    let base = json!({ "service": "sonarr", "item": { "title": "T", "tvdbId": 1 }, "rootFolderPath": "/tv", "qualityProfileId": 1 });
    let (err, v) = call(&s, "arr_add", base.clone()).await;
    assert!(!err, "{v}");
    let body = sent(&log, "POST").pop().unwrap().body;
    let flags = body["addOptions"]
        .as_object()
        .unwrap_or_else(|| panic!("addOptions absent : {body}"));
    assert!(
        flags.values().filter_map(Value::as_bool).all(|b| !b) && !flags.is_empty(),
        "recherche déclenchée par défaut : {body}"
    );
    // avec search:true explicite, la recherche est demandée
    let mut asked = base;
    asked["search"] = json!(true);
    let (err, v) = call(&s, "arr_add", asked).await;
    assert!(!err, "{v}");
    let body = sent(&log, "POST").pop().unwrap().body;
    assert!(
        body["addOptions"]
            .as_object()
            .unwrap()
            .values()
            .any(|b| b == true),
        "{body}"
    );
}

// ---- Transports : stdio (Session directe, ci-dessus) et HTTP (vrai binaire, vraie requête) ----

fn init() -> Value {
    json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {} })
}

#[tokio::test]
async fn http_transport_serves_tools_list_and_a_read_in_loopback() {
    let (url, log) = fake_server().await;
    let port = free_port();
    let mut child = spawn_http(&env_for(&url, &[]), "127.0.0.1", port);
    assert!(
        wait_up(&mut child, port).await,
        "le serveur HTTP ne démarre pas"
    );
    let (st, body) = http(port, "POST", "/mcp", &[], None, Some(&init()))
        .await
        .unwrap();
    assert_eq!(st, 200, "{body}");
    assert!(rpc_body(&body)["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["name"] == "arr_list"));
    let call = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": { "name": "arr_list", "arguments": { "service": "sonarr", "resource": "series" } } });
    let (st, body) = http(port, "POST", "/mcp", &[], None, Some(&call))
        .await
        .unwrap();
    assert_eq!(st, 200, "{body}");
    assert_eq!(sent(&log, "GET").last().unwrap().path, "/api/v3/series");
    // fermé par défaut aussi par ce transport
    let (_, body) = http(port, "POST", "/mcp", &[], None, Some(&init()))
        .await
        .unwrap();
    let parsed = rpc_body(&body);
    let listed: Vec<&str> = parsed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(
        !listed.contains(&"arr_delete") && !listed.contains(&"arr_add") && listed.len() == 13,
        "{listed:?}"
    );
}

#[tokio::test]
async fn http_transport_health_hides_version_and_services() {
    let (url, _) = fake_server().await;
    let port = free_port();
    let mut child = spawn_http(&env_for(&url, &[]), "127.0.0.1", port);
    assert!(wait_up(&mut child, port).await);
    let (st, body) = http(port, "GET", "/health", &[], None, None).await.unwrap();
    assert_eq!(st, 200, "{body}");
    let low = body.to_lowercase();
    for leak in [
        "version", "sonarr", "radarr", "lidarr", "prowlarr", "tracearr", "9.9.9",
    ] {
        assert!(!low.contains(leak), "/health révèle « {leak} » : {body}");
    }
}

#[tokio::test]
async fn http_transport_refuses_a_foreign_host_header() {
    let (url, _) = fake_server().await;
    let port = free_port();
    let mut child = spawn_http(&env_for(&url, &[]), "127.0.0.1", port);
    assert!(wait_up(&mut child, port).await);
    let (st, _) = http(
        port,
        "POST",
        "/mcp",
        &[],
        Some("evil.example"),
        Some(&init()),
    )
    .await
    .unwrap();
    assert_eq!(st, 403);
}

#[tokio::test]
async fn http_transport_outside_loopback_requires_a_token() {
    let (url, _) = fake_server().await;
    // sans jeton : le serveur refuse de démarrer
    let port = free_port();
    let mut child = spawn_http(&env_for(&url, &[]), "0.0.0.0", port);
    assert!(
        !wait_up(&mut child, port).await,
        "écoute hors boucle locale sans jeton"
    );
    // avec jeton : refusé sans, refusé avec un mauvais, accepté avec le bon
    let port = free_port();
    let mut child = spawn_http(
        &env_for(&url, &[("ARR_MCP_TOKEN", "t0ken")]),
        "0.0.0.0",
        port,
    );
    assert!(wait_up(&mut child, port).await);
    let (st, _) = http(port, "POST", "/mcp", &[], None, Some(&init()))
        .await
        .unwrap();
    assert_eq!(st, 401);
    let (st, _) = http(
        port,
        "POST",
        "/mcp",
        &[("Authorization", "Bearer nope")],
        None,
        Some(&init()),
    )
    .await
    .unwrap();
    assert_eq!(st, 401);
    let (st, body) = http(
        port,
        "POST",
        "/mcp",
        &[("Authorization", "Bearer t0ken")],
        None,
        Some(&init()),
    )
    .await
    .unwrap();
    assert_eq!(st, 200, "{body}");
}

#[tokio::test]
async fn http_transport_cannot_reach_a_write_when_closed() {
    let (url, log) = fake_server().await;
    let port = free_port();
    let mut child = spawn_http(&env_for(&url, &[]), "127.0.0.1", port);
    assert!(wait_up(&mut child, port).await);
    let call = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": { "name": "arr_command", "arguments": { "service": "sonarr", "name": "RefreshSeries" } } });
    let (_, body) = http(port, "POST", "/mcp", &[], None, Some(&call))
        .await
        .unwrap();
    assert!(
        rpc_body(&body).get("error").is_some() || rpc_body(&body)["result"]["isError"] == true,
        "{body}"
    );
    assert_eq!(writes_received(&log), 0);
}

// ---- Revue indépendante : chaque test a d'abord été constaté ROUGE ----

/// F1 : en dry-run, `would_send` recopiait l'objet lu sans masque.
#[tokio::test]
async fn dry_run_update_never_shows_a_secret_in_would_send() {
    let (s, log) = start(&[WRITES]).await;
    let (err, v) = call(
        &s,
        "arr_update",
        json!({ "service": "sonarr", "resource": "indexer", "id": 7, "changes": { "enable": false }, "dry_run": true }),
    )
    .await;
    assert!(!err && v["dry_run"] == true, "{v}");
    assert!(!v["would_send"].as_array().unwrap().is_empty(), "{v}");
    assert!(
        !v.to_string().contains(SECRET),
        "secret dans le dry-run : {v}"
    );
    assert_eq!(writes_received(&log), 0);
}

/// F2 : noms de secrets des notifications et de config/host ; `keyword` reste lisible.
#[tokio::test]
async fn redaction_covers_notification_and_host_secrets() {
    let (s, _) = start(&[]).await;
    for path in ["notification", "config/host"] {
        let (err, v) = call(
            &s,
            "arr_api_get",
            json!({ "service": "sonarr", "path": path }),
        )
        .await;
        assert!(!err, "{path}: {v}");
        assert!(
            !v.to_string().contains(SECRET),
            "{path} : secret non masqué : {v}"
        );
        assert!(
            v.to_string().contains("kept"),
            "{path} : faux positif : {v}"
        );
    }
}

/// F3 : recopier un objet lu (donc masqué) dans `changes` renverrait `[redacted]` au service.
#[tokio::test]
async fn update_refuses_to_write_back_the_redaction_mask() {
    let (s, log) = start(&[WRITES]).await;
    let (err, got) = call(
        &s,
        "arr_get",
        json!({ "service": "sonarr", "resource": "indexer", "id": 7 }),
    )
    .await;
    assert!(!err, "{got}");
    let fields = got["item"]["fields"].clone();
    assert_eq!(fields[0]["value"], "[redacted]", "{got}");
    let before = received(&log);
    for changes in [
        json!({ "fields": fields }),
        json!({ "password": "[redacted]" }),
    ] {
        let (err, v) = call(
            &s,
            "arr_update",
            json!({ "service": "sonarr", "resource": "indexer", "id": 7, "changes": changes }),
        )
        .await;
        assert!(err && v["error_type"] == "invalid_input", "{v}");
        assert!(v.to_string().contains("redacted"), "{v}");
    }
    assert_eq!(received(&log), before, "{:?}", log.lock().unwrap());
}

/// F4 : `details` d'une erreur (corps du 400) et arguments de l'audit masqués.
#[tokio::test]
async fn error_details_and_audit_are_redacted() {
    let dir = tempfile::tempdir().unwrap();
    let audit = dir.path().join("audit.jsonl");
    let (s, log) = start(&[WRITES, ("ARR_AUDIT_LOG", audit.to_str().unwrap())]).await;
    let (err, v) = call(
        &s,
        "arr_update",
        json!({ "service": "sonarr", "resource": "indexer", "id": 8, "changes": { "monitored": false } }),
    )
    .await;
    assert!(err && v["error_type"] == "validation_failed", "{v}");
    assert_eq!(sent(&log, "PUT").len(), 1);
    assert!(!v.to_string().contains(SECRET), "details non masqués : {v}");
    let (err, v) = call(
        &s,
        "arr_create",
        json!({ "service": "sonarr", "resource": "indexer", "body": { "name": "n", "fields": [{ "name": "botToken", "value": SECRET }] } }),
    )
    .await;
    assert!(!err, "{v}");
    let text = std::fs::read_to_string(&audit).unwrap();
    assert_eq!(text.lines().count(), 2, "{text}");
    assert!(!text.contains(SECRET), "secret dans l'audit : {text}");
}

/// F5 : stdin fermé pendant des appels lents : toutes les réponses doivent sortir.
#[tokio::test]
async fn stdio_answers_every_request_after_stdin_closes() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (url, _) = fake_server().await;
    let mut child = spawn_stdio(&env_for(&url, &[]));
    let mut input = String::from("{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-06-18\",\"capabilities\":{}}}\n");
    for id in 2..5 {
        input.push_str(&format!("{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"tools/call\",\"params\":{{\"name\":\"arr_api_get\",\"arguments\":{{\"service\":\"sonarr\",\"path\":\"slow\"}}}}}}\n"));
    }
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(input.as_bytes()).await.unwrap();
    drop(stdin);
    let mut out = String::new();
    let mut stdout = child.stdout.take().unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(30),
        stdout.read_to_string(&mut out),
    )
    .await
    .expect("le processus ne s'arrête pas")
    .unwrap();
    let ids: Vec<i64> = out
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|m| m["result"]["isError"] != true && m.get("result").is_some())
        .filter_map(|m| m["id"].as_i64())
        .collect();
    for id in 1..5 {
        assert!(ids.contains(&id), "réponse {id} perdue : {out}");
    }
}

/// F7 : Tracearr absent ou hors liste blanche : refus avant de solliciter un humain.
#[tokio::test]
async fn terminate_checks_tracearr_and_policy_before_asking_a_human() {
    for (extra, kind) in [
        (
            vec![("TRACEARR_URL", ""), ("TRACEARR_API_KEY", "")],
            "not_configured",
        ),
        (
            vec![("ARR_WRITE_SERVICES", "sonarr")],
            "service_not_allowed",
        ),
    ] {
        let mut env = vec![WRITES, DELETE];
        env.extend(extra);
        let (s, log, mut rx) = start_with(&env, true).await;
        rpc(
            &s,
            "initialize",
            json!({ "protocolVersion": "2025-06-18", "capabilities": { "elicitation": {} } }),
        )
        .await;
        let s2 = s.clone();
        let task = tokio::spawn(async move {
            call(
                &s2,
                "arr_terminate_stream",
                json!({ "id": "00000000-0000-4000-8000-000000000001" }),
            )
            .await
        });
        let done = tokio::time::timeout(std::time::Duration::from_secs(2), task).await;
        assert!(rx.try_recv().is_err(), "{kind} : un humain a été sollicité");
        let (err, v) = done.expect("en attente d'une confirmation").unwrap();
        assert!(err && v["error_type"] == kind, "{v}");
        assert_eq!(received(&log), 0);
    }
}

/// F9 : une confirmation sans réponse échoue fermé après ARR_ELICIT_TIMEOUT, comptée dans les stats.
#[tokio::test]
async fn an_unanswered_confirmation_times_out_closed_and_is_counted() {
    let (s, log, mut rx) = start_with(&[WRITES, DELETE, ("ARR_ELICIT_TIMEOUT", "1")], true).await;
    rpc(
        &s,
        "initialize",
        json!({ "protocolVersion": "2025-06-18", "capabilities": { "elicitation": {} } }),
    )
    .await;
    let s2 = s.clone();
    let task = tokio::spawn(async move {
        call(
            &s2,
            "arr_delete",
            json!({ "service": "sonarr", "resource": "series", "id": 1 }),
        )
        .await
    });
    assert_eq!(rx.recv().await.unwrap()["method"], "elicitation/create");
    let (err, v) = tokio::time::timeout(std::time::Duration::from_secs(5), task)
        .await
        .expect("pas de délai sur l'elicitation")
        .unwrap();
    assert!(err && v["error_type"] == "confirmation_required", "{v}");
    assert!(sent(&log, "DELETE").is_empty());
    let (_, st) = call(&s, "arr_stats", json!({})).await;
    assert_eq!(st["elicitation"]["timeouts"], 1, "{st}");
}

/// F10 : arr_api_get ne lit pas le disque de l'hôte *arr (navigateur de fichiers, sauvegardes, journaux, images).
#[tokio::test]
async fn api_get_refuses_host_file_access_before_any_request() {
    let (s, log) = start(&[]).await;
    for path in [
        "filesystem",
        "FileSystem/type",
        "/filesystem/mediafiles/",
        "system/backup",
        "log/file",
        "log/file/sonarr.txt",
        "mediacover/1/poster.jpg",
    ] {
        let (err, v) = call(
            &s,
            "arr_api_get",
            json!({ "service": "sonarr", "path": path, "query": { "path": "/" } }),
        )
        .await;
        assert!(
            err && v["error_type"] == "resource_not_allowed",
            "{path}: {v}"
        );
    }
    assert_eq!(received(&log), 0, "{:?}", log.lock().unwrap());
    let (err, v) = call(
        &s,
        "arr_api_get",
        json!({ "service": "sonarr", "path": "system/status" }),
    )
    .await;
    assert!(!err, "{v}");
}

fn season_search() -> Value {
    json!({ "service": "sonarr", "seriesId": 1, "seasonNumber": 1 })
}

/// Issue #6 : une recherche live lente (GET release, 2 s ici) dépasse ARR_TIMEOUT mais tient dans ARR_SEARCH_TIMEOUT.
#[tokio::test]
async fn a_live_search_may_outlast_arr_timeout() {
    let (s, log) = start(&[("ARR_TIMEOUT", "1"), ("ARR_SEARCH_TIMEOUT", "5")]).await;
    let (err, v) = call(&s, "arr_releases", season_search()).await;
    assert!(!err, "{v}");
    assert_eq!(v["result"]["returned"], 1, "{v}");
    let gets = sent(&log, "GET");
    assert_eq!(gets.len(), 1, "{gets:?}");
    assert_eq!(gets[0].path, "/api/v3/release");
    assert!(gets[0].query.contains("seasonNumber=1"), "{gets:?}");
}

/// Quand le délai d'une recherche live est dépassé, l'erreur dit la valeur, la variable et la phase ; une seule requête part.
#[tokio::test]
async fn a_live_search_timeout_names_its_limit_and_variable() {
    let (s, log) = start(&[("ARR_TIMEOUT", "1"), ("ARR_SEARCH_TIMEOUT", "1")]).await;
    let (err, v) = call(&s, "arr_releases", season_search()).await;
    assert!(err && v["error_type"] == "timeout", "{v}");
    let msg = v["error"].as_str().unwrap();
    assert!(msg.starts_with("sonarr did not answer in time"), "{v}");
    assert!(
        msg.contains("total limit 1 s") && msg.contains("ARR_SEARCH_TIMEOUT"),
        "{v}"
    );
    assert!(v["hint"].as_str().unwrap().contains("MCP client"), "{v}");
    assert_eq!(sent(&log, "GET").len(), 1, "aucun nouvel essai");
}

/// ARR_SEARCH_TIMEOUT ne rallonge que les recherches live : un autre GET lent garde ARR_TIMEOUT.
#[tokio::test]
async fn arr_search_timeout_does_not_extend_other_calls() {
    let (s, _) = start(&[("ARR_TIMEOUT", "1"), ("ARR_SEARCH_TIMEOUT", "5")]).await;
    let started = std::time::Instant::now();
    let (err, v) = call(
        &s,
        "arr_api_get",
        json!({ "service": "sonarr", "path": "slow" }),
    )
    .await;
    assert!(err && v["error_type"] == "timeout", "{v}");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
}
