//! Backend simulé du mode test : réponses fictives issues de `test/fixtures/<service>.json`, aucun réseau.
//! Un chemin que le mock ne connaît pas est coupé (`TEST_MODE_EGRESS_BLOCKED`) : jamais de repli vers le réel.

use crate::client::Method;
use crate::client::MockFn;
use crate::error::{ArrError, ErrorType, Result};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::Arc;

const SOURCES: [(&str, &str); 5] = [
    ("sonarr", include_str!("../test/fixtures/sonarr.json")),
    ("radarr", include_str!("../test/fixtures/radarr.json")),
    ("lidarr", include_str!("../test/fixtures/lidarr.json")),
    ("prowlarr", include_str!("../test/fixtures/prowlarr.json")),
    ("tracearr", include_str!("../test/fixtures/tracearr.json")),
];

/// `/api/v3/series/1?x=2` -> `series/1` ; les préfixes d'API (`api/v3`, `api/v2/public`...) sont retirés.
fn relative(path: &str) -> String {
    let p = path.split('?').next().unwrap_or("").trim_matches('/');
    let p = match p.find("api/") {
        Some(i) => &p[i + 4..],
        None => p,
    };
    let mut seg: Vec<&str> = p.split('/').collect();
    if seg.first().is_some_and(|s| {
        s.len() > 1 && s.starts_with('v') && s[1..].bytes().all(|b| b.is_ascii_digit())
    }) {
        seg.remove(0);
    }
    if seg.first() == Some(&"public") {
        seg.remove(0);
    }
    seg.join("/")
}

fn blocked(service: &str, method: &Method, path: &str) -> ArrError {
    ArrError::new(
        ErrorType::TestModeEgressBlocked,
        format!("TEST_MODE_EGRESS_BLOCKED: {method} {service} {path} is not in the test fixtures."),
    )
}

fn not_found(service: &str, path: &str) -> ArrError {
    crate::client::absent(service, path)
}

fn answer(
    data: &HashMap<String, Map<String, Value>>,
    service: &str,
    method: &Method,
    path: &str,
    body: Option<&Value>,
) -> Result<Value> {
    let fx = data
        .get(service)
        .ok_or_else(|| blocked(service, method, path))?;
    let rel = relative(path);
    let seg: Vec<&str> = rel.split('/').collect();
    if *method == Method::GET {
        if let Some(v) = fx.get(&rel) {
            return Ok(v.clone());
        }
        // `ressource/{id}` : élément d'une liste (ou de `data` pour Tracearr), sinon objet absent.
        if seg.len() == 2 && seg[1].bytes().all(|b| b.is_ascii_digit()) {
            let list = fx
                .get(seg[0])
                .ok_or_else(|| blocked(service, method, path))?;
            let id: u64 = seg[1].parse().unwrap_or(0);
            return list
                .as_array()
                .and_then(|a| a.iter().find(|o| o["id"] == id))
                .cloned()
                .ok_or_else(|| not_found(service, path));
        }
        // Tracearr : `media/{ref}`.
        if seg.len() == 2 && fx.contains_key(seg[0]) && fx[seg[0]].is_object() {
            return Ok(fx[seg[0]].clone());
        }
        return Err(blocked(service, method, path));
    }
    // Écritures : seulement sur une ressource connue du service, ou command / release / test / terminate.
    let known = fx.contains_key(seg[0])
        || matches!(seg[0], "command" | "release" | "queue")
        || rel.ends_with("/terminate");
    if !known {
        return Err(blocked(service, method, path));
    }
    if *method == Method::DELETE {
        return Ok(json!({}));
    }
    if rel.ends_with("/terminate") {
        return Ok(json!({ "success": true }));
    }
    let mut out = body.and_then(Value::as_object).cloned().unwrap_or_default();
    if seg[0] == "command" {
        out.insert("status".into(), json!("queued"));
    }
    out.entry("id").or_insert(json!(1));
    Ok(Value::Object(out))
}

/// Le mock partagé par le client en mode test (fixtures lues une fois).
pub fn mock() -> MockFn {
    let data: HashMap<String, Map<String, Value>> = SOURCES
        .iter()
        .map(|(s, t)| {
            (
                s.to_string(),
                serde_json::from_str::<Value>(t)
                    .ok()
                    .and_then(|v| v.as_object().cloned())
                    .unwrap_or_default(),
            )
        })
        .collect();
    Arc::new(move |service, method, path, _query, body| answer(&data, service, method, path, body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixtures_parse_and_answer() {
        let m = mock();
        let s = m("sonarr", &Method::GET, "/api/v3/series", &[], None).unwrap();
        assert_eq!(s[0]["id"], 1);
        assert_eq!(
            m("sonarr", &Method::GET, "/api/v3/series/1", &[], None).unwrap()["id"],
            1
        );
        assert!(m("sonarr", &Method::GET, "/api/v3/series/99", &[], None).is_err());
        assert!(m(
            "tracearr",
            &Method::GET,
            "/api/v2/public/streams",
            &[],
            None
        )
        .is_ok());
        let c = m(
            "radarr",
            &Method::POST,
            "/api/v3/command",
            &[],
            Some(&json!({ "name": "RefreshMovie" })),
        )
        .unwrap();
        assert_eq!(c["status"], "queued");
    }

    #[test]
    fn unknown_paths_are_blocked() {
        let m = mock();
        let e = m("sonarr", &Method::GET, "/api/v3/inconnu", &[], None).unwrap_err();
        assert!(e.is(ErrorType::TestModeEgressBlocked));
        assert!(m("sonarr", &Method::POST, "/api/v3/inconnu", &[], None).is_err());
        assert!(m("autre", &Method::GET, "/x", &[], None).is_err());
    }
}
