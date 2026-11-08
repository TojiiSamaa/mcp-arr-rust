//! HTTP « streamable » sans session : chaque requête a sa propre `Session` au-dessus du `Registry`.
//! Sans session il n'y a personne à interroger : pas d'elicitation, une action destructive répond
//! `confirmation_required` (sauf ARR_CONFIRM_ARG, ou l'approbateur simulé du mode test).

use crate::error::{ArrError, ErrorType, Result};
use crate::server::{Registry, Session};
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::Router;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::Arc;

pub struct HttpOptions {
    pub host: String,
    pub port: u16,
    /// Chemin MCP (`MCP_PATH`, défaut `/mcp`).
    pub path: String,
    /// Jeton Bearer du serveur MCP (`ARR_MCP_TOKEN`). Obligatoire hors boucle locale.
    pub token: Option<String>,
    pub allowed_hosts: Vec<String>,
    pub cors_origins: Vec<String>,
}

pub fn is_loopback(host: &str) -> bool {
    host == "localhost" || host == "::1" || host.starts_with("127.")
}

/// Comparaison en temps constant (la longueur du jeton attendu fixe le nombre de tours).
pub fn token_matches(header: Option<&str>, token: &str) -> bool {
    let given = header.unwrap_or("").as_bytes();
    let wanted = format!("Bearer {token}");
    let wanted = wanted.as_bytes();
    let mut diff = (given.len() ^ wanted.len()) as u8 | u8::from(given.len() != wanted.len());
    for (i, w) in wanted.iter().enumerate() {
        diff |= given.get(i).copied().unwrap_or(0) ^ w;
    }
    diff == 0
}

struct AppState {
    registry: Arc<Registry>,
    token: Option<String>,
    path: String,
    /// None : écoute hors boucle locale sans ARR_MCP_ALLOWED_HOSTS, où un jeton est obligatoire ; le contrôle
    /// de Host ne protège alors de rien et casserait docker -p, un reverse proxy ou un nom DNS.
    allowed: Option<HashSet<String>>,
    cors: Vec<String>,
}

fn deny(status: StatusCode, message: &str) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "application/json")],
        json!({ "error": message }).to_string(),
    )
        .into_response()
}

async fn handle(
    State(state): State<Arc<AppState>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let origin = headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let cors_ok = origin
        .as_ref()
        .filter(|o| state.cors.iter().any(|c| c == "*" || c == *o))
        .cloned();
    let mut resp = route(&state, method, uri, headers, body).await;
    if let Some(o) = cors_ok.and_then(|o| HeaderValue::from_str(&o).ok()) {
        let h = resp.headers_mut();
        h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, o);
        h.insert(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_static("authorization, content-type, mcp-protocol-version"),
        );
        h.insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_static("POST, OPTIONS"),
        );
    }
    resp
}

async fn route(
    state: &AppState,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    // Vérifier Host arrête le DNS rebinding depuis une page web.
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if state.allowed.as_ref().is_some_and(|a| !a.contains(host)) {
        return deny(
            StatusCode::FORBIDDEN,
            "Host not allowed. See ARR_MCP_ALLOWED_HOSTS.",
        );
    }
    if method == Method::OPTIONS {
        return StatusCode::NO_CONTENT.into_response();
    }
    // Santé : rien d'autre que « ok » (ni version, ni services) ; pas de jeton pour les sondes.
    if uri.path() == "/health" && method == Method::GET {
        return (
            [(header::CONTENT_TYPE, "application/json")],
            json!({ "status": "ok" }).to_string(),
        )
            .into_response();
    }
    if let Some(token) = &state.token {
        if !token_matches(
            headers
                .get(header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok()),
            token,
        ) {
            return deny(StatusCode::UNAUTHORIZED, "Invalid or missing bearer token.");
        }
    }
    if uri.path() != state.path {
        return deny(StatusCode::NOT_FOUND, "Not found.");
    }
    if method != Method::POST {
        return deny(
            StatusCode::METHOD_NOT_ALLOWED,
            "This server keeps no session: only POST is supported.",
        );
    }
    let Ok(message) = serde_json::from_slice::<Value>(&body) else {
        return (StatusCode::BAD_REQUEST, [(header::CONTENT_TYPE, "application/json")], json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": "Parse error" } }).to_string()).into_response();
    };
    let session = Session::new(state.registry.clone(), None);
    let reply = match message {
        Value::Array(batch) => {
            let replies: Vec<Value> =
                futures::future::join_all(batch.into_iter().map(|m| session.handle(m)))
                    .await
                    .into_iter()
                    .flatten()
                    .collect();
            (!replies.is_empty()).then_some(Value::Array(replies))
        }
        single => session.handle(single).await,
    };
    match reply {
        Some(r) => ([(header::CONTENT_TYPE, "application/json")], r.to_string()).into_response(),
        None => StatusCode::ACCEPTED.into_response(),
    }
}

/// Prépare l'écoute (refus sans jeton hors boucle locale) ; renvoie le port réel et le serveur à lancer.
pub async fn bind(
    registry: Arc<Registry>,
    options: HttpOptions,
) -> Result<(u16, impl std::future::Future<Output = Result<()>>)> {
    let HttpOptions {
        host,
        port,
        path,
        token,
        allowed_hosts,
        cors_origins,
    } = options;
    if token.is_none() && !is_loopback(&host) {
        return Err(ArrError::new(ErrorType::ConfigError, format!("Refusing to listen on {host} without a token: anyone on the network could drive your services."))
            .hint("Set ARR_MCP_TOKEN, or listen on 127.0.0.1."));
    }
    if !path.starts_with('/') || path == "/health" {
        return Err(ArrError::new(
            ErrorType::ConfigError,
            format!("MCP_PATH: \"{path}\" must start with / and must not be /health."),
        ));
    }
    let listener = tokio::net::TcpListener::bind((host.as_str(), port))
        .await
        .map_err(|e| {
            ArrError::new(
                ErrorType::ConfigError,
                format!("Cannot listen on {host}:{port}: {e}"),
            )
        })?;
    let bound = listener.local_addr().map(|a| a.port()).unwrap_or(port);
    let shown = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.clone()
    };
    let explicit = !allowed_hosts.is_empty();
    let mut allowed: HashSet<String> = allowed_hosts.into_iter().collect();
    for name in [shown.as_str(), "localhost", "127.0.0.1", "[::1]"] {
        if name == shown || is_loopback(&host) {
            allowed.insert(format!("{name}:{bound}"));
        }
    }
    crate::log::info(&format!("listening on http://{shown}:{bound}{path}"));
    let allowed = (is_loopback(&host) || explicit).then_some(allowed);
    let app = Router::new()
        .fallback(handle)
        .with_state(Arc::new(AppState {
            registry,
            token,
            path,
            allowed,
            cors: cors_origins,
        }));
    Ok((bound, async move {
        axum::serve(listener, app).await.map_err(|e| {
            ArrError::new(
                ErrorType::InternalError,
                format!("HTTP server stopped: {e}"),
            )
        })
    }))
}

pub async fn serve(registry: Arc<Registry>, options: HttpOptions) -> Result<()> {
    bind(registry, options).await?.1.await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_comparison() {
        assert!(token_matches(Some("Bearer abc"), "abc"));
        assert!(!token_matches(Some("Bearer abd"), "abc"));
        assert!(!token_matches(Some("Bearer abcd"), "abc"));
        assert!(!token_matches(Some("Bearer ab"), "abc"));
        assert!(!token_matches(Some("abc"), "abc"));
        assert!(!token_matches(None, "abc"));
        // Longueurs dont la différence est un multiple de 256 : ne doit pas passer.
        let long = format!("Bearer abc{}", "x".repeat(256));
        assert!(!token_matches(Some(&long), "abc"));
    }

    #[test]
    fn loopback() {
        assert!(is_loopback("127.0.0.1") && is_loopback("::1") && is_loopback("localhost"));
        assert!(!is_loopback("0.0.0.0") && !is_loopback("192.0.2.1"));
    }

    fn reg() -> Arc<Registry> {
        let cfg = crate::config::load_config(
            &[("ARR_TEST_MODE".to_string(), "1".to_string())]
                .into_iter()
                .collect(),
        )
        .unwrap();
        Registry::new(crate::runtime::Runtime::with_mock(cfg, None).unwrap())
    }

    fn opts(host: &str, token: Option<&str>, path: &str) -> HttpOptions {
        HttpOptions {
            host: host.into(),
            port: 0,
            path: path.into(),
            token: token.map(String::from),
            allowed_hosts: vec![],
            cors_origins: vec![],
        }
    }

    #[tokio::test]
    async fn refuses_open_network_without_token() {
        assert!(bind(reg(), opts("0.0.0.0", None, "/mcp")).await.is_err());
        assert!(bind(reg(), opts("127.0.0.1", None, "/health"))
            .await
            .is_err());
        assert!(bind(reg(), opts("127.0.0.1", None, "/mcp")).await.is_ok());
    }

    async fn call(port: u16, req: &str) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        s.write_all(req.as_bytes()).await.unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).await.unwrap();
        out
    }

    #[tokio::test]
    async fn health_is_bare_and_token_is_checked() {
        let (port, server) = bind(reg(), opts("127.0.0.1", Some("s3cret"), "/rpc"))
            .await
            .unwrap();
        tokio::spawn(server);
        let host = format!("127.0.0.1:{port}");
        let health = call(
            port,
            &format!("GET /health HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"),
        )
        .await;
        assert!(
            health.starts_with("HTTP/1.1 200") && health.ends_with(r#"{"status":"ok"}"#),
            "{health}"
        );
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
        let post = |auth: &str| {
            format!("POST /rpc HTTP/1.1\r\nHost: {host}\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
        };
        assert!(call(port, &post("")).await.starts_with("HTTP/1.1 401"));
        assert!(call(port, &post("Authorization: Bearer s3cret\r\n"))
            .await
            .starts_with("HTTP/1.1 200"));
        let bad_host = format!(
            "GET /health HTTP/1.1\r\nHost: evil.example:{port}\r\nConnection: close\r\n\r\n"
        );
        assert!(call(port, &bad_host).await.starts_with("HTTP/1.1 403"));
    }
}
