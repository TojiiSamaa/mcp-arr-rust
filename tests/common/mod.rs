//! Outils communs des tests de bout en bout : un faux serveur qui répond comme sonarr/radarr/lidarr/prowlarr/tracearr
//! et ENREGISTRE chaque requête reçue (méthode, chemin, requête, corps, en-têtes d'authentification), plus les
//! helpers pour piloter le serveur MCP (session directe ou processus HTTP réel).
#![allow(dead_code)]

use arrmcp::config::{load_config, Env};
use arrmcp::runtime::Runtime;
use arrmcp::server::{Registry, Session};
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::Router;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Valeur que le faux serveur glisse dans ses réponses : elle ne doit jamais ressortir telle quelle.
pub const SECRET: &str = "supersecret-value";

/// Une requête reçue par le faux serveur.
#[derive(Clone, Debug)]
pub struct Rec {
    pub method: String,
    pub path: String,
    pub query: String,
    pub body: Value,
    pub api_key: Option<String>,
    pub bearer: Option<String>,
}

pub type Log = Arc<Mutex<Vec<Rec>>>;

fn reply(status: StatusCode, v: Value) -> Response {
    (
        status,
        [("content-type", "application/json")],
        v.to_string(),
    )
        .into_response()
}

async fn fake(
    State(log): State<Log>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let header = |n: &str| {
        headers
            .get(n)
            .and_then(|v| v.to_str().ok())
            .map(String::from)
    };
    let path = uri.path().to_string();
    log.lock().unwrap().push(Rec {
        method: method.to_string(),
        path: path.clone(),
        query: uri.query().unwrap_or("").to_string(),
        body: serde_json::from_slice(&body).unwrap_or(Value::Null),
        api_key: header("x-api-key"),
        bearer: header("authorization"),
    });
    let last = path.rsplit('/').next().unwrap_or("");
    // Un 400 qui recopie la clé envoyée (comme le font les validations *arr) : `details` doit être masqué.
    if method == Method::PUT && path.ends_with("/indexer/8") {
        return reply(
            StatusCode::BAD_REQUEST,
            json!([{ "propertyName": "ApiKey", "attemptedValue": SECRET, "apiKey": SECRET }]),
        );
    }
    if method != Method::GET {
        return match (method.as_str(), last) {
            ("POST", "command") => reply(
                StatusCode::CREATED,
                json!({ "id": 5, "name": "RefreshSeries", "status": "queued" }),
            ),
            ("DELETE", _) => reply(StatusCode::OK, json!({})),
            _ => reply(StatusCode::OK, json!({ "id": 9, "title": "T" })),
        };
    }
    let item = |id: i64| json!({ "id": id, "title": "T", "monitored": true, "tags": [], "apiKey": SECRET, "password": SECRET });
    match last {
        "status" if path.ends_with("/system/status") => reply(
            StatusCode::OK,
            json!({ "appName": "fake", "version": "9.9.9" }),
        ),
        "health" => reply(StatusCode::OK, json!([])),
        // Service lent : la réponse arrive après plus de 2 s.
        "slow" => {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            reply(StatusCode::OK, json!([]))
        }
        // Recherche live (GET release) : les indexeurs mettent plus de 2 s à répondre.
        "release" => {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            reply(StatusCode::OK, json!([item(1)]))
        }
        // Formes de secrets des *arr : champs de réglage d'un indexeur, d'une notification, configuration de l'hôte.
        "7" if path.ends_with("/indexer/7") => reply(
            StatusCode::OK,
            json!({ "id": 7, "name": "idx", "enable": true, "password": SECRET,
                "fields": [{ "name": "apiKey", "value": SECRET }, { "name": "baseUrl", "value": "http://idx.example" }] }),
        ),
        "notification" => reply(
            StatusCode::OK,
            json!([{ "id": 1, "name": "discord", "fields": [
                { "name": "webHookUrl", "value": SECRET }, { "name": "botToken", "value": SECRET },
                { "name": "authToken", "value": SECRET }, { "name": "accessToken", "value": SECRET },
                { "name": "keyword", "value": "kept" }] }]),
        ),
        "host" => reply(
            StatusCode::OK,
            json!({ "apiKey": SECRET, "proxyPassword": SECRET, "sslCertPassword": SECRET,
                "passwordConfirmation": SECRET, "proxyUsername": "kept", "keyword": "kept" }),
        ),
        // Une page HTML en 404 : ne doit jamais être recopiée dans l'erreur.
        "nothing" => (StatusCode::NOT_FOUND, "<html>nope</html>").into_response(),
        "404" => reply(StatusCode::NOT_FOUND, json!({ "message": "NotFound" })),
        "lookup" => reply(
            StatusCode::OK,
            json!([{ "title": "T", "tvdbId": 1, "tmdbId": 1, "foreignArtistId": "a1", "artistName": "T" }]),
        ),
        "queue" | "history" | "missing" | "cutoff" => reply(
            StatusCode::OK,
            json!({ "page": 1, "pageSize": 10, "totalRecords": 0, "records": [] }),
        ),
        _ if path.contains("/public/") => reply(
            StatusCode::OK,
            json!({ "data": [{ "id": "s1", "apiKey": SECRET }], "nextCursor": null }),
        ),
        _ if last.parse::<i64>().is_ok() => reply(StatusCode::OK, item(last.parse().unwrap())),
        _ => reply(StatusCode::OK, json!([item(1)])),
    }
}

/// Lance le faux serveur ; renvoie son URL et son journal.
pub async fn fake_server() -> (String, Log) {
    let log: Log = Arc::default();
    let app = Router::new().fallback(fake).with_state(log.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (url, log)
}

pub const KEYS: [(&str, &str); 5] = [
    ("SONARR", "key-sonarr"),
    ("RADARR", "key-radarr"),
    ("LIDARR", "key-lidarr"),
    ("PROWLARR", "key-prowlarr"),
    ("TRACEARR", "trr_pub_test"),
];

/// Environnement des cinq services pointés sur le faux serveur, puis les variables du test (qui l'emportent).
pub fn env_for(url: &str, extra: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = KEYS
        .iter()
        .flat_map(|(n, k)| {
            [
                (format!("{n}_URL"), url.to_string()),
                (format!("{n}_API_KEY"), k.to_string()),
            ]
        })
        .collect();
    env.extend(extra.iter().map(|(k, v)| (k.to_string(), v.to_string())));
    env
}

pub async fn start(extra: &[(&str, &str)]) -> (Arc<Session>, Log) {
    let (s, log, _) = start_with(extra, false).await;
    (s, log)
}

/// `channel` : la session a un canal sortant (stdio), donc peut envoyer une requête d'elicitation.
pub async fn start_with(
    extra: &[(&str, &str)],
    channel: bool,
) -> (
    Arc<Session>,
    Log,
    tokio::sync::mpsc::UnboundedReceiver<Value>,
) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let (url, log) = fake_server().await;
    let env: Env = env_for(&url, extra).into_iter().collect();
    let rt = Runtime::new(load_config(&env).unwrap()).await.unwrap();
    (
        Session::new(Registry::new(rt), channel.then_some(tx)),
        log,
        rx,
    )
}

pub async fn rpc(s: &Arc<Session>, method: &str, params: Value) -> Value {
    s.handle(json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }))
        .await
        .unwrap()
}

/// Appelle un outil : (`isError`, contenu JSON décodé).
pub async fn call(s: &Arc<Session>, name: &str, args: Value) -> (bool, Value) {
    let r = rpc(s, "tools/call", json!({ "name": name, "arguments": args })).await;
    let res = &r["result"];
    let text = res["content"][0]["text"].as_str().unwrap_or("null");
    (
        res["isError"] == true,
        serde_json::from_str(text).unwrap_or(Value::Null),
    )
}

pub async fn tools(s: &Arc<Session>) -> Vec<Value> {
    rpc(s, "tools/list", json!({})).await["result"]["tools"]
        .as_array()
        .unwrap()
        .clone()
}

pub async fn names(s: &Arc<Session>) -> Vec<String> {
    tools(s)
        .await
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect()
}

/// Outils qui ne sont pas en lecture seule (écriture ou destructifs), d'après les annotations publiées.
pub async fn write_tools(s: &Arc<Session>) -> Vec<String> {
    tools(s)
        .await
        .iter()
        .filter(|t| t["annotations"]["readOnlyHint"] != true)
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect()
}

pub async fn tool_schema(s: &Arc<Session>, name: &str) -> Value {
    tools(s)
        .await
        .into_iter()
        .find(|t| t["name"] == name)
        .unwrap_or_else(|| panic!("outil {name} absent"))["inputSchema"]
        .clone()
}

/// Requêtes reçues pour une méthode donnée.
pub fn sent(log: &Log, method: &str) -> Vec<Rec> {
    log.lock()
        .unwrap()
        .iter()
        .filter(|r| r.method == method)
        .cloned()
        .collect()
}

/// Tout ce qui n'est pas un GET : c'est ce que les défauts fermés et le dry-run doivent garder à zéro.
pub fn writes_received(log: &Log) -> usize {
    log.lock()
        .unwrap()
        .iter()
        .filter(|r| r.method != "GET")
        .count()
}

pub fn received(log: &Log) -> usize {
    log.lock().unwrap().len()
}

// ---- Transport HTTP : le vrai binaire, une vraie requête TCP ----

pub fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Lance le binaire en HTTP. `host` : adresse d'écoute (`127.0.0.1` ou `0.0.0.0`).
pub fn spawn_http(env: &[(String, String)], host: &str, port: u16) -> tokio::process::Child {
    tokio::process::Command::new(env!("CARGO_BIN_EXE_arrmcp"))
        .env_clear()
        .envs(env.iter().cloned())
        .env("MCP_TRANSPORT", "http")
        .env("HOST", host)
        .env("PORT", port.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
}

/// Lance le binaire en stdio, entrée et sortie en tubes.
pub fn spawn_stdio(env: &[(String, String)]) -> tokio::process::Child {
    tokio::process::Command::new(env!("CARGO_BIN_EXE_arrmcp"))
        .env_clear()
        .envs(env.iter().cloned())
        .args(["--transport", "stdio"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
}

/// Requête HTTP/1.1 brute (aucune dépendance) : (statut, corps). `host` remplace l'en-tête Host.
pub async fn http(
    port: u16,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    host: Option<&str>,
    body: Option<&Value>,
) -> std::io::Result<(u16, String)> {
    let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port)).await?;
    let body = body.map(|b| b.to_string()).unwrap_or_default();
    let host = host
        .map(String::from)
        .unwrap_or_else(|| format!("127.0.0.1:{port}"));
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\n", body.len());
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    s.write_all(format!("{req}\r\n{body}").as_bytes()).await?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).await?;
    let text = String::from_utf8_lossy(&raw).to_string();
    let status = text
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    Ok((
        status,
        text.split_once("\r\n\r\n")
            .map(|(_, b)| b.to_string())
            .unwrap_or_default(),
    ))
}

/// Attend que le port réponde (ou que le processus s'arrête : alors `false`).
pub async fn wait_up(child: &mut tokio::process::Child, port: u16) -> bool {
    for _ in 0..100 {
        if child.try_wait().unwrap().is_some() {
            return false;
        }
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    false
}

/// Extrait le JSON-RPC d'un corps HTTP (JSON direct ou événement SSE `data:`), tolérant au découpage en morceaux.
pub fn rpc_body(body: &str) -> Value {
    let start = body.find('{').unwrap_or(0);
    let end = body.rfind('}').map(|e| e + 1).unwrap_or(body.len());
    serde_json::from_str(&body[start..end]).unwrap_or(Value::Null)
}
