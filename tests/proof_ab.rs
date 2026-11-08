//! Preuves A (structurelle) et B (dynamique) du mode test, plus la mutation qui doit faire échouer A.
//! A : aucun fichier de src/ hors client.rs ne touche au réseau, à un socket ou à un processus.
//! B : tous les outils de `tools/list` appelés en mode test vers un écouteur sentinelle : 0 connexion, 0 octet.

use arrmcp::config::{load_config, Env};
use arrmcp::runtime::Runtime;
use arrmcp::server::{Registry, Session};
use serde_json::{json, Value};
use std::io::Read;
use std::net::TcpListener;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

// ---------------------------------------------------------------- preuve A

/// Motifs interdits hors de client.rs (liste à ajuster si un nouveau chemin réseau apparaît).
/// Écouter (serveur HTTP du MCP) n'est pas sortir ; les modules `#[cfg(test)]` (fin de fichier) ne sont pas du code livré.
const FORBIDDEN: [&str; 8] = [
    "reqwest",
    "TcpStream",
    "UdpSocket",
    "process::Command",
    "hyper::client",
    "hyper_util::client",
    "native_tls",
    "std::os::unix::net",
];

/// Retourne « fichier:ligne motif » pour chaque occurrence hors client.rs (commentaires `//` ignorés).
fn violations(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(violations(&path));
            continue;
        }
        if path.extension().is_none_or(|e| e != "rs")
            || path.file_name().is_some_and(|n| n == "client.rs")
        {
            continue;
        }
        // Only the item that follows `#[cfg(test)]` is skipped (brace counting), never the rest of the file.
        let (mut in_test, mut depth, mut opened) = (false, 0i32, false);
        for (i, line) in std::fs::read_to_string(&path).unwrap().lines().enumerate() {
            if !in_test && line.trim() == "#[cfg(test)]" {
                (in_test, depth, opened) = (true, 0, false);
                continue;
            }
            if in_test {
                depth += line.matches('{').count() as i32 - line.matches('}').count() as i32;
                opened |= line.contains('{');
                if (opened && depth <= 0) || (!opened && line.trim_end().ends_with(';')) {
                    in_test = false;
                }
                continue;
            }
            if line.trim_start().starts_with("//") {
                continue;
            }
            for pat in FORBIDDEN {
                if line.contains(pat) {
                    out.push(format!("{}:{} {pat}", path.display(), i + 1));
                }
            }
        }
    }
    out
}

fn src_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

#[test]
fn proof_a_no_network_outside_client() {
    let found = violations(&src_dir());
    assert!(
        found.is_empty(),
        "network code outside client.rs: {found:#?}"
    );
    assert!(
        src_dir().join("client.rs").exists(),
        "client.rs missing: the single network point must exist"
    );
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let p = e.unwrap().path();
        let dest = to.join(p.file_name().unwrap());
        if p.is_dir() {
            copy_dir(&p, &dest);
        } else {
            std::fs::copy(&p, &dest).unwrap();
        }
    }
}

/// Mutation : on injecte un vrai appel réseau hors de client.rs dans une COPIE de src/ ; la preuve A doit échouer.
#[test]
fn mutation_a_catches_an_injected_network_call() {
    for payload in [
        "pub fn leak() { let _ = reqwest::get(\"http://example.invalid/\"); }",
        "pub fn leak() { let _ = std::net::TcpStream::connect(\"127.0.0.1:9\"); }",
        "pub fn leak() { let _ = std::process::Command::new(\"curl\"); }",
    ] {
        let tmp = tempfile::tempdir().unwrap();
        copy_dir(&src_dir(), tmp.path());
        assert!(
            violations(tmp.path()).is_empty(),
            "the untouched copy must be clean"
        );
        let victim = tmp.path().join("testmode.rs");
        let text = std::fs::read_to_string(&victim).unwrap();
        std::fs::write(&victim, format!("{payload}\n{text}")).unwrap();
        assert!(
            !violations(tmp.path()).is_empty(),
            "proof A did not catch: {payload}"
        );
        // Same call appended AFTER a file's `#[cfg(test)]` module: still shipped code, must be caught.
        let tmp = tempfile::tempdir().unwrap();
        copy_dir(&src_dir(), tmp.path());
        let victim = tmp.path().join("tools_read.rs");
        let text = std::fs::read_to_string(&victim).unwrap();
        assert!(
            text.contains("#[cfg(test)]"),
            "tools_read.rs must keep a test module for this case"
        );
        std::fs::write(&victim, format!("{text}\n{payload}\n")).unwrap();
        assert!(
            !violations(tmp.path()).is_empty(),
            "proof A did not catch after the test module: {payload}"
        );
    }
}

// ---------------------------------------------------------------- preuve B

/// Écouteur sentinelle : compte les connexions et les octets reçus.
struct Sentinel {
    addr: String,
    conns: Arc<AtomicUsize>,
    bytes: Arc<AtomicUsize>,
}

fn sentinel() -> Sentinel {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let (conns, bytes) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let (c, b) = (conns.clone(), bytes.clone());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            c.fetch_add(1, Ordering::SeqCst);
            let b = b.clone();
            std::thread::spawn(move || {
                let mut s = stream;
                let mut buf = [0u8; 4096];
                while let Ok(n) = s.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    b.fetch_add(n, Ordering::SeqCst);
                }
            });
        }
    });
    Sentinel { addr, conns, bytes }
}

async fn start(sent: &Sentinel, approver: &str) -> Arc<Session> {
    let base = format!("http://{}", sent.addr);
    let mut env: Env = Env::new();
    for s in ["SONARR", "RADARR", "LIDARR", "PROWLARR", "TRACEARR"] {
        env.insert(format!("{s}_URL"), base.clone());
        env.insert(format!("{s}_API_KEY"), "cle-fictive-0123456789".into());
    }
    for (k, v) in [
        ("ARR_TEST_MODE", "1"),
        ("ARR_TEST_APPROVER", approver),
        ("ARR_ALLOW_WRITES", "true"),
        ("ARR_ALLOW_DELETE", "true"),
        ("ARR_ELICIT", "all"),
    ] {
        env.insert(k.into(), v.into());
    }
    let rt = Runtime::new(load_config(&env).unwrap()).await.unwrap();
    Session::new(Registry::new(rt), None)
}

async fn rpc(s: &Arc<Session>, method: &str, params: Value) -> Value {
    s.handle(json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }))
        .await
        .unwrap()
}

struct Tool {
    name: String,
    required: Vec<String>,
    props: Value,
    read_only: bool,
}

async fn tools(s: &Arc<Session>) -> Vec<Tool> {
    rpc(s, "tools/list", json!({})).await["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| Tool {
            name: t["name"].as_str().unwrap().to_string(),
            required: t["inputSchema"]["required"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
            props: t["inputSchema"]["properties"].clone(),
            read_only: t["annotations"]["readOnlyHint"] == true,
        })
        .collect()
}

/// Valeurs plausibles et fictives pour chaque argument obligatoire ; surcharges par outil quand le défaut ne suffit pas.
fn sample_args(t: &Tool) -> Value {
    let mut a = serde_json::Map::new();
    for k in &t.required {
        let schema = &t.props[k];
        let v = match k.as_str() {
            "service" => json!(match t.name.as_str() {
                "arr_terminate_stream" => "tracearr",
                "arr_search_indexers" | "arr_test" => "prowlarr",
                _ => "sonarr",
            }),
            "resource" => json!(if t.name == "arr_create" {
                "tag"
            } else if t.name == "arr_test" {
                "indexer"
            } else {
                "series"
            }),
            "id" if schema["type"] == "string" => json!("00000000-0000-4000-8000-000000000001"),
            "id" | "indexerId" | "seriesId" | "movieId" | "artistId" | "albumId" | "episodeId" => {
                json!(1)
            }
            "term" => json!("exemple"),
            "name" => json!(if t.name == "arr_command" {
                "RefreshSeries"
            } else {
                "exemple"
            }),
            "guid" => json!("guid-fictif-1"),
            "kind" => json!("missing"),
            "path" => json!("system/status"),
            _ => match schema["type"].as_str() {
                Some("integer") | Some("number") => json!(1),
                Some("boolean") => json!(true),
                Some("array") => json!([]),
                Some("object") => json!({}),
                _ => schema["enum"].get(0).cloned().unwrap_or(json!("exemple")),
            },
        };
        a.insert(k.clone(), v);
    }
    match t.name.as_str() {
        "arr_releases" => a.insert("episodeId".into(), json!(1)),
        "arr_add" => a.insert(
            "item".into(),
            json!({ "title": "Série fictive", "tvdbId": 1 }),
        ),
        "arr_update" => a.insert("changes".into(), json!({ "monitored": false })),
        "arr_create" | "arr_test" => a.insert("body".into(), json!({ "label": "exemple" })),
        "arr_delete" | "arr_queue_remove" | "arr_terminate_stream" => {
            a.insert("confirm".into(), json!(true))
        }
        _ => None,
    };
    Value::Object(a)
}

/// Appelle un outil et renvoie (isError, contenu JSON décodé).
async fn call(s: &Arc<Session>, name: &str, args: Value) -> (bool, Value) {
    let r = rpc(s, "tools/call", json!({ "name": name, "arguments": args })).await;
    let res = &r["result"];
    let text = res["content"][0]["text"].as_str().unwrap_or("null");
    (
        res["isError"] == true,
        serde_json::from_str(text).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn proof_b_every_tool_answers_without_touching_the_network() {
    let sent = sentinel();
    let s = start(&sent, "approve").await;
    let all = tools(&s).await;
    assert!(
        all.len() >= 20,
        "tools/list looks truncated: {} tools",
        all.len()
    );
    let mut called = 0usize;
    for t in &all {
        let (err, body) = call(&s, &t.name, sample_args(t)).await;
        assert!(!err, "{} failed in test mode: {body}", t.name);
        assert_eq!(
            body["test_mode"], true,
            "{} answer lacks test_mode:true: {body}",
            t.name
        );
        called += 1;
    }
    assert_eq!(called, all.len());
    assert_eq!(
        sent.conns.load(Ordering::SeqCst),
        0,
        "a connection reached the sentinel"
    );
    assert_eq!(
        sent.bytes.load(Ordering::SeqCst),
        0,
        "bytes reached the sentinel"
    );
    // Stats cohérentes : aucun appel ni requête réels, des requêtes simulées, aucune coupe (les fixtures couvrent tout).
    let (_, st) = call(&s, "arr_stats", json!({})).await;
    assert_eq!(st["calls"]["real"], 0, "stats: {st}");
    assert_eq!(st["network"]["requests_real"], 0, "stats: {st}");
    assert!(
        st["calls"]["test"].as_u64().unwrap_or(0) >= all.len() as u64 - 1,
        "stats: {st}"
    );
    assert!(
        st["network"]["requests_mock"].as_u64().unwrap_or(0) > 0,
        "stats: {st}"
    );
    assert_eq!(st["network"]["egress_blocked"], 0, "stats: {st}");
}

/// Quatre scénarios pour CHAQUE outil à validation : approve -> exécute ; deny, timeout -> rien ; absent -> confirmation_required.
#[tokio::test]
async fn approver_scenarios_for_every_validated_tool() {
    // (approbateur, l'appel doit-il s'exécuter, type d'erreur attendu)
    let cases: [(&str, bool, &str); 4] = [
        ("approve", true, ""),
        ("deny", false, "user_cancelled"),
        ("timeout", false, "user_cancelled"),
        ("absent", false, "confirmation_required"),
    ];
    let sent = sentinel();
    let names: Vec<(String, Value)> = {
        let s = start(&sent, "approve").await;
        tools(&s)
            .await
            .iter()
            .filter(|t| !t.read_only)
            .map(|t| (t.name.clone(), sample_args(t)))
            .collect()
    };
    assert!(
        names.len() >= 9,
        "expected the write and destructive tools, got {}",
        names.len()
    );
    for (approver, runs, kind) in cases {
        for (name, args) in &names {
            let s = start(&sent, approver).await;
            let (err, body) = call(&s, name, args.clone()).await;
            let (_, st) = call(&s, "arr_stats", json!({})).await;
            let executed = st["network"]["requests_mock"].as_u64().unwrap_or(0) > 0;
            if runs {
                assert!(
                    !err && body["test_mode"] == true && executed,
                    "{name}/{approver}: {body}"
                );
            } else {
                assert!(
                    err && body["error_type"] == kind,
                    "{name}/{approver}: {body}"
                );
                assert!(
                    !executed,
                    "{name}/{approver}: the mock ran although the approval failed"
                );
            }
        }
    }
    assert_eq!(sent.conns.load(Ordering::SeqCst), 0);
}

/// Le mode test ne se pilote pas par un appel : aucun outil n'a d'argument qui le touche, et un appel qui essaie est sans effet.
#[tokio::test]
async fn no_tool_can_toggle_test_mode() {
    let sent = sentinel();
    let s = start(&sent, "approve").await;
    for t in tools(&s).await {
        let schema = serde_json::to_string(&t.props).unwrap().to_lowercase();
        assert!(
            !schema.contains("test_mode")
                && !schema.contains("testmode")
                && !schema.contains("approver"),
            "{} exposes a test-mode knob",
            t.name
        );
        let mut args = sample_args(&t);
        for k in [
            "test_mode",
            "ARR_TEST_MODE",
            "approver",
            "ARR_TEST_APPROVER",
        ] {
            args[k] = json!(false);
        }
        let (_, body) = call(&s, &t.name, args).await;
        assert_eq!(body["test_mode"], true, "{} left test mode: {body}", t.name);
    }
    assert_eq!(sent.conns.load(Ordering::SeqCst), 0);
}

/// Sans mock en mode test, toute requête est coupée ET comptée ; un chemin hors fixtures répond `TEST_MODE_EGRESS_BLOCKED`.
#[tokio::test]
async fn cut_is_counted_and_unknown_path_is_blocked() {
    let sent = sentinel();
    let s = start(&sent, "approve").await;
    let (err, body) = call(
        &s,
        "arr_api_get",
        json!({ "service": "sonarr", "path": "chemin-inconnu" }),
    )
    .await;
    assert!(
        err && body["error_type"] == "TEST_MODE_EGRESS_BLOCKED",
        "{body}"
    );
    // Runtime sans mock : le client ne construit rien et coupe.
    let mut env = Env::new();
    env.insert("ARR_TEST_MODE".into(), "1".into());
    let rt = Runtime::with_mock(load_config(&env).unwrap(), None).unwrap();
    let s2 = Session::new(Registry::new(rt), None);
    let (err, body) = call(&s2, "arr_status", json!({})).await;
    let (_, st) = call(&s2, "arr_stats", json!({})).await;
    assert!(
        st["network"]["egress_blocked"].as_u64().unwrap_or(0) >= 1,
        "{body} {st}"
    );
    assert_eq!(st["network"]["requests_real"], 0);
    let _ = err;
    assert_eq!(sent.conns.load(Ordering::SeqCst), 0);
}
