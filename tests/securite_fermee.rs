//! Politique de sécurité fermée par défaut : compléments à tests/e2e.rs (même faux serveur qui enregistre tout).
//! Ici : l'interrupteur de suppression seul n'ouvre rien, la liste blanche s'applique à CHAQUE outil d'écriture,
//! et aucune clé d'API ne sort sur stdout ni sur stderr (journal complet) même quand le service est injoignable.
mod common;
use common::*;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn refused(r: &Value) -> bool {
    r.get("error").is_some() || r["result"]["isError"] == true
}

/// Un appel par outil non lecture, sur le service donné (arr_terminate_stream vise Tracearr).
fn calls_on(service: &str) -> Vec<(&'static str, Value)> {
    vec![
        (
            "arr_add",
            json!({ "service": service, "item": { "title": "T" }, "rootFolderPath": "/x", "qualityProfileId": 1 }),
        ),
        (
            "arr_update",
            json!({ "service": service, "resource": "series", "id": 1, "changes": { "monitored": false } }),
        ),
        (
            "arr_command",
            json!({ "service": service, "name": "RefreshSeries" }),
        ),
        (
            "arr_grab_release",
            json!({ "service": service, "guid": "g", "indexerId": 1 }),
        ),
        (
            "arr_create",
            json!({ "service": service, "resource": "tag", "body": { "label": "x" } }),
        ),
        (
            "arr_test",
            json!({ "service": service, "resource": "indexer", "body": { "name": "i" } }),
        ),
        (
            "arr_delete",
            json!({ "service": service, "resource": "series", "id": 1 }),
        ),
        ("arr_queue_remove", json!({ "service": service, "id": 1 })),
    ]
}

#[tokio::test]
async fn delete_switch_alone_opens_nothing() {
    // ARR_ALLOW_DELETE sans ARR_ALLOW_WRITES : l'état reste entièrement fermé.
    let (s, log) = start(&[("ARR_ALLOW_DELETE", "true")]).await;
    assert!(
        write_tools(&s).await.is_empty(),
        "{:?}",
        write_tools(&s).await
    );
    for (tool, args) in calls_on("sonarr") {
        let r = rpc(&s, "tools/call", json!({ "name": tool, "arguments": args })).await;
        assert!(
            refused(&r),
            "{tool} accepté avec ARR_ALLOW_DELETE seul : {r}"
        );
    }
    assert_eq!(writes_received(&log), 0, "{:?}", log.lock().unwrap());
}

#[tokio::test]
async fn allowlist_refuses_every_write_tool_on_another_service_before_any_request() {
    let (s, log) = start(&[
        ("ARR_ALLOW_WRITES", "true"),
        ("ARR_ALLOW_DELETE", "true"),
        ("ARR_ELICIT", "off"),
        ("ARR_WRITE_SERVICES", "radarr"),
    ])
    .await;
    for (tool, args) in calls_on("sonarr") {
        let (err, v) = call(&s, tool, args).await;
        assert!(
            err && v["error_type"] == "service_not_allowed",
            "{tool} : refus attendu par la liste blanche : {v}"
        );
    }
    assert_eq!(
        received(&log),
        0,
        "le service a reçu : {:?}",
        log.lock().unwrap()
    );
    // la liste blanche ne bloque pas le service nommé
    let (err, v) = call(
        &s,
        "arr_command",
        json!({ "service": "radarr", "name": "RefreshMovie" }),
    )
    .await;
    assert!(!err, "{v}");
}

#[tokio::test]
async fn api_keys_never_leave_on_stdout_or_stderr_even_when_a_service_is_down() {
    let key_down = "key-leak-down-0123456789";
    let key_up = "key-leak-up-9876543210";
    let (url, _log) = fake_server().await;
    let dead = {
        let p = free_port();
        format!("http://127.0.0.1:{p}")
    };
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_arrmcp"))
        .env_clear()
        .envs([
            ("SONARR_URL", dead.as_str()),
            ("SONARR_API_KEY", key_down),
            ("RADARR_URL", url.as_str()),
            ("RADARR_API_KEY", key_up),
            ("LOG_LEVEL", "debug"),
        ])
        .args(["--transport", "stdio"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let reqs = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{}}}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"arr_status","arguments":{"service":"sonarr"}}}),
        json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"arr_list","arguments":{"service":"radarr","resource":"movie"}}}),
        json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"arr_command","arguments":{"service":"sonarr","name":"RefreshSeries"}}}),
        json!({"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"arr_api_get","arguments":{"service":"sonarr","path":"series"}}}),
    ];
    let mut input = String::new();
    for r in &reqs {
        input.push_str(&r.to_string());
        input.push('\n');
    }
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(input.as_bytes()).await.unwrap();
    drop(stdin);
    let (mut out, mut err) = (String::new(), String::new());
    let mut so = child.stdout.take().unwrap();
    let mut se = child.stderr.take().unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let _ = tokio::join!(so.read_to_string(&mut out), se.read_to_string(&mut err));
    })
    .await
    .expect("le processus ne s'arrête pas");
    assert!(
        out.contains("\"id\":3"),
        "pas de réponse pour le service injoignable : {out}"
    );
    for k in [key_down, key_up] {
        assert!(!out.contains(k), "clé dans stdout : {out}");
        assert!(!err.contains(k), "clé dans stderr : {err}");
    }
}
