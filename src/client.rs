//! Client REST des services *arr et de Tracearr : SEUL fichier qui cite `reqwest`, SEUL point réseau (`send`).
//! En mode test le vrai client n'est jamais construit : tout passe par un `MockFn`, et sans mock la requête
//! est coupée (`TEST_MODE_EGRESS_BLOCKED`, comptée dans les stats).
//! Les redirections ne sont pas suivies : un POST redirigé perd son corps, mieux vaut corriger l'URL.

use crate::config::Config;
use crate::error::{ArrError, ErrorType, Result};
use crate::services::{self, Auth};
use crate::stats::Stats;
use serde_json::{json, Map, Value};
use std::sync::Arc;
use std::time::Duration;

/// Réexporté pour que le reste du code n'ait pas à citer `reqwest`.
pub use reqwest::Method;

/// Délai de connexion : fixe, aucune variable ne le règle.
const CONNECT_TIMEOUT_SECS: u64 = 10;

/// Le client MCP a son propre délai, distinct de ceux d'arrmcp : un délai plus court coupe l'appel avant la réponse.
const CLIENT_TIMEOUT_NOTE: &str =
    "The MCP client has its own timeout, separate from arrmcp's: keep it at least as long.";

/// Réponse simulée : (service, méthode, chemin complet `/api/v3/series/1`, requête, corps).
pub type MockFn = Arc<
    dyn Fn(&str, &Method, &str, &[(String, String)], Option<&Value>) -> Result<Value> + Send + Sync,
>;

#[derive(Clone)]
enum Backend {
    Real(reqwest::Client),
    Mock(MockFn),
    /// Mode test sans mock : tout est coupé.
    Blocked,
}

#[derive(Clone)]
pub struct Client {
    backend: Backend,
    cfg: Arc<Config>,
    stats: Option<Arc<Stats>>,
}

/// Normalise un chemin relatif au préfixe d'API du service : `/series/3/` donne `series/3`.
/// Refuse tout ce qui sortirait de l'API (schéma, `..`, `%`, requête, ancre, antislash, préfixe `api/` recopié).
pub fn api_path(raw: &str) -> Result<String> {
    let bad = |why: &str| {
        ArrError::new(ErrorType::InvalidInput, format!("Invalid path \"{raw}\": {why}."))
            .hint("Use a path relative to the service API prefix, for example series/12 or system/status. Put filters in `query`.")
    };
    if raw.contains("://")
        || raw.contains("..")
        || raw.contains('\\')
        || raw.contains('?')
        || raw.contains('#')
        || raw.contains('%')
    {
        return Err(bad("no scheme, '..', '%', '?', '#' or backslash allowed"));
    }
    if raw.chars().any(|c| c.is_control() || c == ' ') {
        return Err(bad("no space or control character allowed"));
    }
    let p = raw.trim().trim_matches('/');
    if p.is_empty() {
        return Err(bad("empty path"));
    }
    if p == "api" || p.starts_with("api/") {
        return Err(bad("do not repeat the /api/vN prefix, the service adds it"));
    }
    if p.contains("//") {
        return Err(bad("empty segment"));
    }
    Ok(p.to_string())
}

/// Chemin complet côté service (`/api/v3/series/3`), préfixe choisi par la table des services.
pub fn full_path(service: &str, raw: &str) -> Result<String> {
    let def = services::get(service).ok_or_else(|| unknown_service(service))?;
    let rel = api_path(raw)?;
    Ok(format!("{}/{rel}", def.prefix_for(&rel)))
}

/// Un objet de filtres devient des paires `clé=valeur` ; un tableau répète la clé (`id=1&id=2`).
pub fn query_pairs(map: &Map<String, Value>) -> Vec<(String, String)> {
    fn one(v: &Value) -> Option<String> {
        match v {
            Value::String(s) => Some(s.clone()),
            Value::Bool(b) => Some(b.to_string()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        }
    }
    let mut out = Vec::new();
    for (k, v) in map {
        match v {
            Value::Null => {}
            Value::Array(a) => out.extend(a.iter().filter_map(one).map(|s| (k.clone(), s))),
            other => out.extend(one(other).map(|s| (k.clone(), s))),
        }
    }
    out
}

fn unknown_service(service: &str) -> ArrError {
    ArrError::new(
        ErrorType::InvalidInput,
        format!("Unknown service \"{service}\"."),
    )
    .hint(format!("Known services: {}.", services::names().join(", ")))
}

impl Client {
    pub fn new(cfg: &Config, mock: Option<MockFn>) -> Result<Client> {
        let backend = match (cfg.test_mode, mock) {
            (_, Some(m)) => Backend::Mock(m),
            (true, None) => Backend::Blocked,
            (false, None) => Backend::Real(
                reqwest::Client::builder()
                    .danger_accept_invalid_certs(!cfg.verify_ssl)
                    .redirect(reqwest::redirect::Policy::none())
                    .timeout(Duration::from_secs(cfg.timeout_secs))
                    .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
                    .user_agent(concat!("arrmcp/", env!("CARGO_PKG_VERSION")))
                    .build()
                    .map_err(|e| {
                        ArrError::new(
                            ErrorType::ConfigError,
                            format!("Cannot build the HTTP client: {e}"),
                        )
                    })?,
            ),
        };
        Ok(Client {
            backend,
            cfg: Arc::new(cfg.clone()),
            stats: None,
        })
    }

    /// Branche les compteurs partagés (coupes réseau, requêtes réelles/simulées).
    pub fn with_stats(mut self, stats: Arc<Stats>) -> Self {
        self.stats = Some(stats);
        self
    }

    pub fn is_real(&self) -> bool {
        matches!(self.backend, Backend::Real(_))
    }

    /// `path` est relatif au préfixe d'API du service (voir `api_path`).
    pub async fn request(
        &self,
        service: &str,
        method: Method,
        path: &str,
        query: &[(String, String)],
        body: Option<&Value>,
    ) -> Result<Value> {
        let full = full_path(service, path)?;
        let conf = self.cfg.service(service).ok_or_else(|| {
            let def = services::get(service);
            ArrError::new(
                ErrorType::NotConfigured,
                format!("{service} is not configured on this server: nothing was sent."),
            )
            .hint(
                def.map(|d| {
                    format!(
                        "Do not retry. The operator can set {} and {}.",
                        d.url_env, d.key_env
                    )
                })
                .unwrap_or_default(),
            )
        })?;
        self.send(service, &conf.url, &conf.key, method, &full, query, body)
            .await
    }

    pub async fn get(
        &self,
        service: &str,
        path: &str,
        query: &[(String, String)],
    ) -> Result<Value> {
        self.request(service, Method::GET, path, query, None).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn send(
        &self,
        service: &str,
        base: &str,
        key: &str,
        method: Method,
        full: &str,
        query: &[(String, String)],
        body: Option<&Value>,
    ) -> Result<Value> {
        let http = match &self.backend {
            Backend::Mock(m) => {
                let result = m(service, &method, full, query, body);
                if let Some(s) = &self.stats {
                    // Hors fixtures = coupe réseau (comptée), pas une requête simulée.
                    match &result {
                        Err(e) if e.is(ErrorType::TestModeEgressBlocked) => s.egress_blocked(),
                        _ => s.request(false),
                    }
                }
                return result;
            }
            Backend::Blocked => {
                if let Some(s) = &self.stats {
                    s.egress_blocked();
                }
                return Err(ArrError::new(ErrorType::TestModeEgressBlocked, format!("TEST_MODE_EGRESS_BLOCKED: {method} {service} {full} was not sent (test mode, no mock).")));
            }
            Backend::Real(h) => h,
        };
        if let Some(s) = &self.stats {
            s.request(true);
        }
        let def = services::get(service).ok_or_else(|| unknown_service(service))?;
        let url = format!("{base}{full}");
        let limit = total_limit(&self.cfg, &method, full);
        let mut req = http
            .request(method.clone(), &url)
            .timeout(Duration::from_secs(limit.0))
            .header("Accept", "application/json")
            .query(query);
        req = match def.auth {
            Auth::ApiKeyHeader(h) => req.header(h, key),
            Auth::Bearer => req.header("Authorization", format!("Bearer {key}")),
        };
        if let Some(b) = body {
            req = req.json(b);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| transport_error(&e, service, limit))?;
        let status = resp.status();
        let location = resp
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .map(String::from);
        let text = resp
            .text()
            .await
            .map_err(|e| transport_error(&e, service, limit))?;
        if status.is_redirection() {
            let env = def.url_env;
            return Err(ArrError::new(ErrorType::BadResponse, format!("{service} redirected the request to {}.", location.as_deref().unwrap_or("another address")))
                .status(status.as_u16())
                .hint(format!("Set {env} to the address the service redirects to (base URL, https://). Redirects are not followed: a redirected write would lose its body.")));
        }
        let parsed: Option<Value> = serde_json::from_str(&text).ok();
        if status.is_success() {
            return match (status.as_u16(), parsed) {
                (204, _) => Ok(Value::Null),
                (_, Some(v)) => Ok(v),
                (_, None) if text.trim().is_empty() => Ok(Value::Null),
                (_, None) => Err(not_json(service, &text, status.as_u16())),
            };
        }
        Err(http_error(
            service,
            status.as_u16(),
            parsed,
            &text,
            &method,
            full,
        ))
    }
}

/// Recherche live : le service interroge les indexeurs avant de répondre (`GET release`, `GET search` de Prowlarr).
fn is_live_search(method: &Method, full: &str) -> bool {
    *method == Method::GET && (full.ends_with("/release") || full.ends_with("/search"))
}

/// Délai total d'une requête (secondes) et variable qui le fixe : ARR_TIMEOUT, sauf pour une recherche live qui prend
/// le plus grand d'ARR_TIMEOUT et d'ARR_SEARCH_TIMEOUT (0 = ARR_TIMEOUT seul).
fn total_limit(cfg: &Config, method: &Method, full: &str) -> (u64, &'static str) {
    if is_live_search(method, full) && cfg.search_timeout_secs >= cfg.timeout_secs {
        (cfg.search_timeout_secs, "ARR_SEARCH_TIMEOUT")
    } else {
        (cfg.timeout_secs, "ARR_TIMEOUT")
    }
}

/// Dit quel délai a sauté (connexion fixe, ou durée totale et sa variable) : « élever ARR_TIMEOUT » ne sert à rien
/// si le client MCP coupe avant.
fn timeout_error(service: &str, connecting: bool, (secs, var): (u64, &str)) -> ArrError {
    let (limit, advice) = if connecting {
        (
            format!("connection limit {CONNECT_TIMEOUT_SECS} s, not configurable"),
            format!("Check that {service} is up and reachable."),
        )
    } else {
        (
            format!("total limit {secs} s, set by {var}"),
            format!("Raise {var}, or narrow the query."),
        )
    };
    ArrError::new(
        ErrorType::Timeout,
        format!("{service} did not answer in time ({limit})."),
    )
    .hint(format!("{advice} {CLIENT_TIMEOUT_NOTE}"))
}

fn transport_error(e: &reqwest::Error, service: &str, limit: (u64, &str)) -> ArrError {
    let msg = format!("{e:?}");
    if e.is_timeout() {
        timeout_error(service, e.is_connect(), limit)
    } else if msg.to_ascii_lowercase().contains("certificate") {
        ArrError::new(ErrorType::TlsError, "TLS certificate check failed.")
            .hint("Fix the certificate, or set VERIFY_SSL=false for a lab (insecure).")
    } else {
        // Ni adresse ni `Display` complet : l'URL peut porter des paramètres.
        ArrError::new(ErrorType::Unreachable, format!("Cannot reach {service}."))
            .hint("Check the service URL and that it is running.")
    }
}

fn not_json(service: &str, text: &str, status: u16) -> ArrError {
    let html = text.trim_start().starts_with('<');
    // Une page HTML n'est jamais recopiée (ni début, ni titre).
    ArrError::new(
        ErrorType::BadResponse,
        format!(
            "{service} answered {status} with {}.",
            if html {
                "an HTML page"
            } else {
                "something that is not JSON"
            }
        ),
    )
    .hint(if html {
        "The URL probably points to a login page, a proxy or the web UI base, not to the API."
    } else {
        "Check the service URL."
    })
}

fn http_error(
    service: &str,
    status: u16,
    body: Option<Value>,
    text: &str,
    method: &Method,
    full: &str,
) -> ArrError {
    let detail = body
        .as_ref()
        .and_then(|b| {
            b.get("message")
                .or_else(|| b.get("error"))
                .or_else(|| b.get("detail"))
        })
        .and_then(Value::as_str)
        .map(String::from);
    let html = text.trim_start().starts_with('<');
    let shown = detail.unwrap_or_else(|| {
        if html {
            "an HTML page, not an API answer".into()
        } else if body.is_some() {
            // Un corps JSON n'est jamais recopié en texte (il échapperait au masquage) : il va dans `details`.
            "no message from the service (see `details` when present)".into()
        } else {
            text.chars().take(300).collect()
        }
    });
    let (kind, hint): (ErrorType, &str) = match status {
        400 => (ErrorType::ValidationFailed, "The service rejected the values; `details` holds its answer. Do not resend the same payload unchanged."),
        401 => (ErrorType::AuthFailed, "The API key is invalid or missing. Do not retry: the operator has to fix it."),
        403 => (ErrorType::Forbidden, "The key has no permission for this action. Do not retry."),
        404 => (ErrorType::NotFound, "Nothing at this address, or no such object. Check the resource and the id."),
        405 => (ErrorType::BadRequest, "This endpoint does not support that method."),
        409 => (ErrorType::Conflict, "The object already exists or conflicts with another one."),
        s if s >= 500 => (ErrorType::UpstreamError, "The service failed. Retrying later may work."),
        _ => (ErrorType::BadRequest, ""),
    };
    let mut e = ArrError::new(
        kind,
        format!("{service} {method} {full} -> {status}: {shown}"),
    )
    .status(status)
    .hint(hint);
    // 404 JSON = objet absent sur un chemin qui existe (`found: false`) ; 404 HTML = chemin inconnu (pas de détails).
    if matches!(status, 400 | 404 | 409) && !html {
        if let Some(b) = body {
            e = e.details(b);
        }
    }
    e
}

/// Corps d'un `found: false` simulé ou réel : utile aux mocks pour signaler un objet absent.
pub fn absent(service: &str, full: &str) -> ArrError {
    ArrError::new(
        ErrorType::NotFound,
        format!("{service} GET {full} -> 404: not found"),
    )
    .status(404)
    .details(json!({ "message": "NotFound" }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{load_config, Env};

    fn cfg(pairs: &[(&str, &str)]) -> Config {
        let e: Env = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        load_config(&e).unwrap()
    }

    #[test]
    fn paths_are_normalised_and_confined() {
        assert_eq!(api_path("/series/3/").unwrap(), "series/3");
        assert_eq!(api_path("system/status").unwrap(), "system/status");
        for bad in [
            "",
            "/",
            "http://x/api",
            "a/../b",
            "..",
            "a?b=1",
            "a b",
            "a//b",
            "a\\b",
            "x#y",
            "%2e%2e/x",
            "api/v3/series",
            "/api",
        ] {
            assert!(api_path(bad).is_err(), "{bad}");
        }
        assert_eq!(full_path("sonarr", "series/3").unwrap(), "/api/v3/series/3");
        assert_eq!(full_path("lidarr", "artist").unwrap(), "/api/v1/artist");
        assert_eq!(
            full_path("tracearr", "stats").unwrap(),
            "/api/v1/public/stats"
        );
        assert_eq!(
            full_path("tracearr", "streams").unwrap(),
            "/api/v2/public/streams"
        );
        assert!(full_path("plex", "x").is_err());
    }

    #[test]
    fn arrays_repeat_the_key() {
        let m = json!({ "id": [1, 2], "name": "r", "x": null, "ok": true, "o": {"a": 1} });
        let mut q = query_pairs(m.as_object().unwrap());
        q.sort();
        assert_eq!(
            q,
            vec![
                ("id".into(), "1".into()),
                ("id".into(), "2".into()),
                ("name".into(), "r".into()),
                ("ok".into(), "true".into())
            ]
        );
    }

    /// Issue #6 : une recherche live a son délai total, jamais sous ARR_TIMEOUT ; 0 = l'ancien comportement.
    #[test]
    fn live_searches_have_their_own_total_limit() {
        for (method, path, live) in [
            (Method::GET, "/api/v3/release", true),
            (Method::GET, "/api/v1/search", true),
            (Method::POST, "/api/v3/release", false),
            (Method::GET, "/api/v3/release/push", false),
            (Method::GET, "/api/v3/series/lookup", false),
        ] {
            assert_eq!(is_live_search(&method, path), live, "{method} {path}");
        }
        let get = |c: &Config, path: &str| total_limit(c, &Method::GET, path);
        let c = cfg(&[]);
        assert_eq!(get(&c, "/api/v3/release"), (120, "ARR_SEARCH_TIMEOUT"));
        assert_eq!(get(&c, "/api/v3/series"), (30, "ARR_TIMEOUT"));
        // Jamais en dessous d'ARR_TIMEOUT.
        let c = cfg(&[("ARR_TIMEOUT", "300")]);
        assert_eq!(get(&c, "/api/v3/release"), (300, "ARR_TIMEOUT"));
        // 0 : l'ancien comportement.
        let c = cfg(&[("ARR_SEARCH_TIMEOUT", "0")]);
        assert_eq!(get(&c, "/api/v3/release"), (30, "ARR_TIMEOUT"));
    }

    /// Un délai dépassé dit lequel, sa valeur et sa variable ; la connexion n'en a pas.
    #[test]
    fn a_timeout_names_its_limit() {
        let total = timeout_error("sonarr", false, (120, "ARR_SEARCH_TIMEOUT"));
        let m = &total.message;
        assert!(m.starts_with("sonarr did not answer in time"));
        assert!(m.contains("total limit 120 s, set by ARR_SEARCH_TIMEOUT"));
        let connect = timeout_error("sonarr", true, (120, "ARR_SEARCH_TIMEOUT"));
        assert!(connect.message.contains("connection limit 10 s"));
        for e in [&total, &connect] {
            assert!(e.is(ErrorType::Timeout));
            assert!(e.hint.as_deref().unwrap().contains("MCP client"));
        }
    }

    #[test]
    fn errors_are_typed() {
        let e = http_error(
            "sonarr",
            400,
            Some(json!([{"propertyName": "path"}])),
            "",
            &Method::POST,
            "/api/v3/series",
        );
        assert!(e.is(ErrorType::ValidationFailed) && e.details.is_some());
        assert!(http_error("sonarr", 401, None, "", &Method::GET, "/x").is(ErrorType::AuthFailed));
        assert!(
            http_error("sonarr", 502, None, "bad gateway", &Method::GET, "/x")
                .is(ErrorType::UpstreamError)
        );
        let html = http_error(
            "sonarr",
            404,
            None,
            "<html><title>secret</title>",
            &Method::GET,
            "/x",
        );
        assert!(html.details.is_none() && !html.message.contains("secret"));
    }

    #[tokio::test]
    async fn test_mode_without_mock_is_blocked_and_counted() {
        let c = cfg(&[("ARR_TEST_MODE", "1")]);
        let stats = Arc::new(Stats::default());
        let client = Client::new(&c, None).unwrap().with_stats(stats.clone());
        assert!(!client.is_real());
        let e = client.get("sonarr", "series", &[]).await.unwrap_err();
        assert!(e.is(ErrorType::TestModeEgressBlocked));
        assert_eq!(stats.snapshot()["network"]["egress_blocked"], 1);
    }

    /// Mode test : un chemin hors fixtures est une coupe réseau comptée, pas une requête simulée.
    #[tokio::test]
    async fn fixture_miss_is_counted_as_egress_blocked() {
        let c = cfg(&[("ARR_TEST_MODE", "1")]);
        let stats = Arc::new(Stats::default());
        let client = Client::new(&c, Some(crate::testmode::mock()))
            .unwrap()
            .with_stats(stats.clone());
        client.get("sonarr", "series", &[]).await.unwrap();
        let e = client.get("sonarr", "nothing/here", &[]).await.unwrap_err();
        assert!(e.is(ErrorType::TestModeEgressBlocked));
        let n = &stats.snapshot()["network"];
        assert_eq!(
            (n["requests_mock"].as_u64(), n["egress_blocked"].as_u64()),
            (Some(1), Some(1)),
            "{n}"
        );
    }

    #[tokio::test]
    async fn mock_gets_the_full_path_and_unconfigured_is_refused() {
        let c = cfg(&[("SONARR_URL", "http://s.example"), ("SONARR_API_KEY", "k")]);
        let mock: MockFn = Arc::new(|s, m, p, _, _| Ok(json!({ "s": s, "m": m.as_str(), "p": p })));
        let client = Client::new(&c, Some(mock)).unwrap();
        let v = client.get("sonarr", "/series/1/", &[]).await.unwrap();
        assert_eq!(v["p"], "/api/v3/series/1");
        assert!(client
            .get("radarr", "movie", &[])
            .await
            .unwrap_err()
            .is(ErrorType::NotConfigured));
        assert!(client
            .get("sonarr", "../x", &[])
            .await
            .unwrap_err()
            .is(ErrorType::InvalidInput));
    }
}
