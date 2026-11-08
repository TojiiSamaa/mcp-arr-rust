#![allow(clippy::result_large_err)]
use arrmcp::config::{load_config, merge_dotenv, process_env, Env};
use arrmcp::error::{ArrError, ErrorType};
use arrmcp::http::{self, HttpOptions};
use arrmcp::runtime::Runtime;
use arrmcp::server::{Registry, Session};
use serde_json::Value;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

const HELP: &str =
    "arrmcp: MCP server for Sonarr, Radarr, Lidarr, Prowlarr and Tracearr (read, guarded write)

Usage: arrmcp [--transport stdio|http] [--host 127.0.0.1] [--port 3000] [--test]
              [--list-tools] [--version] [--help]

Options override the environment, which overrides a .env file in the current directory.
  <SERVICE>_URL, <SERVICE>_API_KEY   a service is configured when both are set
                                     (SONARR, RADARR, LIDARR, PROWLARR, TRACEARR)
  MCP_TRANSPORT=http, PORT, HOST, MCP_PATH (default /mcp)
  ARR_ALLOW_WRITES=true              enable writes (read-only by default)
  ARR_ALLOW_DELETE=true              also enable destructive actions
  ARR_DRY_RUN=true                   writes are previewed, never sent
  ARR_MCP_TOKEN                      HTTP bearer token, required off loopback
  --test / ARR_TEST_MODE=1           test mode: no network, simulated answers
                                     (ARR_TEST_APPROVER=approve|deny|timeout|absent)
See the README for the full list.
";

#[derive(Default)]
struct Cli {
    http: Option<bool>,
    host: Option<String>,
    port: Option<String>,
    test: bool,
    list_tools: bool,
    help: bool,
    version: bool,
}

fn parse_cli() -> Result<Cli, ArrError> {
    let mut cli = Cli::default();
    let mut it = std::env::args().skip(1);
    let bad = |m: String| ArrError::new(ErrorType::ConfigError, m);
    while let Some(a) = it.next() {
        let (flag, inline) = match a.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f.to_string(), Some(v.to_string())),
            _ => (a.clone(), None),
        };
        let mut value = |name: &str| {
            inline
                .clone()
                .or_else(|| it.next())
                .ok_or_else(|| bad(format!("{name} needs a value.")))
        };
        match flag.as_str() {
            "--transport" => match value("--transport")?.as_str() {
                "http" | "streamable-http" => cli.http = Some(true),
                "stdio" => cli.http = Some(false),
                v => return Err(bad(format!("--transport: \"{v}\" is not stdio or http."))),
            },
            "--test" => cli.test = true,
            "--list-tools" => cli.list_tools = true,
            "-h" | "--help" => cli.help = true,
            "-V" | "--version" => cli.version = true,
            "--host" => cli.host = Some(value("--host")?),
            "--port" => cli.port = Some(value("--port")?),
            other => return Err(bad(format!("Unknown option \"{other}\". See --help."))),
        }
    }
    Ok(cli)
}

fn build_env(cli: &Cli) -> Env {
    let mut env = process_env();
    if cli.test {
        env.insert("ARR_TEST_MODE".into(), "1".into());
    }
    if let Ok(text) = std::fs::read_to_string(".env") {
        merge_dotenv(&mut env, &text);
    }
    env
}

async fn run() -> Result<(), ArrError> {
    let cli = parse_cli()?;
    if cli.help {
        print!("{HELP}");
        return Ok(());
    }
    if cli.version {
        println!("arrmcp {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let env = build_env(&cli);
    let cfg = load_config(&env)?;
    arrmcp::log::set_level(&cfg.log_level);
    if cfg.test_mode {
        arrmcp::log::info(&format!(
            "TEST MODE: no network, simulated answers, approver = {}",
            cfg.test_approver.as_str()
        ));
    }
    let (mcp_token, allowed_hosts, cors_origins) = (
        cfg.mcp_token.clone(),
        cfg.allowed_hosts.clone(),
        cfg.cors_origins.clone(),
    );
    let registry = Registry::new(Runtime::new(cfg).await?);

    if cli.list_tools {
        for def in &registry.tools {
            println!("{}\t{}", def.access.as_str(), def.name);
        }
        return Ok(());
    }

    let http_mode = cli.http.unwrap_or_else(|| {
        env.get("MCP_TRANSPORT")
            .is_some_and(|t| t.eq_ignore_ascii_case("http"))
    });
    if http_mode {
        let port_text = cli
            .port
            .or_else(|| env.get("PORT").cloned())
            .unwrap_or_else(|| "3000".into());
        let port: u16 = port_text.parse().ok().filter(|p| *p >= 1).ok_or_else(|| {
            ArrError::new(
                ErrorType::ConfigError,
                format!("--port: \"{port_text}\" is not a valid port."),
            )
        })?;
        let host = cli
            .host
            .or_else(|| env.get("HOST").cloned())
            .unwrap_or_else(|| "127.0.0.1".into());
        let path = env
            .get("MCP_PATH")
            .cloned()
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| "/mcp".into());
        return http::serve(
            registry,
            HttpOptions {
                host,
                port,
                path,
                token: mcp_token,
                allowed_hosts,
                cors_origins,
            },
        )
        .await;
    }

    serve_stdio(registry).await;
    Ok(())
}

async fn serve_stdio(registry: Arc<Registry>) {
    let (tx, mut rx) = mpsc::unbounded_channel::<Value>();
    let writer = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Some(msg) = rx.recv().await {
            let mut line = msg.to_string();
            line.push('\n');
            if stdout.write_all(line.as_bytes()).await.is_err() || stdout.flush().await.is_err() {
                break;
            }
        }
    });
    // Un appel fait au plus une lecture puis une écriture, chacune bornée par ARR_TIMEOUT.
    let drain = std::time::Duration::from_secs(registry.runtime.cfg.timeout_secs * 2 + 5);
    let session = Session::new(registry, Some(tx.clone()));
    let mut tasks = tokio::task::JoinSet::new();
    arrmcp::log::info("ready on stdio");
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            let _ = tx.send(serde_json::json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": "Parse error" } }));
            continue;
        };
        if msg.get("method").is_none() {
            // Réponse du client à une requête du serveur (elicitation).
            session.deliver_response(msg);
            continue;
        }
        let (session, tx) = (session.clone(), tx.clone());
        while tasks.try_join_next().is_some() {}
        tasks.spawn(async move {
            if let Some(reply) = session.handle(msg).await {
                let _ = tx.send(reply);
            }
        });
    }
    // Entrée fermée : plus aucune confirmation ne viendra ; on rend toutes les réponses en cours avant de sortir.
    session.abandon_pending();
    let finished =
        tokio::time::timeout(drain, async { while tasks.join_next().await.is_some() {} }).await;
    if finished.is_err() {
        arrmcp::log::warn("stdin closed: calls still running after the drain delay were abandoned");
        tasks.shutdown().await;
    }
    drop(tx);
    drop(session);
    // Le rédacteur s'arrête quand toutes les réponses sont écrites ; borne si la sortie est bloquée.
    let _ = tokio::time::timeout(std::time::Duration::from_secs(10), writer).await;
}

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        eprintln!(
            "[arrmcp] {}{}",
            err.message,
            err.hint
                .as_ref()
                .map(|h| format!("\n        {h}"))
                .unwrap_or_default()
        );
        std::process::exit(1);
    }
}
