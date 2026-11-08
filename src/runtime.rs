//! Ce qui survit à une connexion MCP : configuration, client, statistiques, journal d'audit.

use crate::client::{Client, MockFn};
use crate::config::Config;
use crate::error::{ArrError, ErrorType, Result};
use crate::stats::Stats;
use serde_json::{json, Map, Value};
use std::io::Write;
use std::sync::Arc;

pub struct Runtime {
    pub cfg: Arc<Config>,
    pub client: Client,
    pub stats: Arc<Stats>,
}

impl Runtime {
    /// En mode test, le mock de `testmode` remplace le réseau ; sinon le vrai client.
    pub async fn new(cfg: Config) -> Result<Arc<Self>> {
        let mock = cfg.test_mode.then(crate::testmode::mock);
        Self::with_mock(cfg, mock)
    }

    /// Construction avec un mock donné (tests) ; `None` en mode test = toute requête coupée.
    pub fn with_mock(cfg: Config, mock: Option<MockFn>) -> Result<Arc<Self>> {
        if let Some(path) = &cfg.audit_log {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .map_err(|e| {
                    ArrError::new(
                        ErrorType::ConfigError,
                        format!("ARR_AUDIT_LOG: cannot write to {path}: {e}"),
                    )
                })?;
        }
        let stats = Arc::new(Stats::default());
        let client = Client::new(&cfg, mock)?.with_stats(stats.clone());
        Ok(Arc::new(Runtime {
            cfg: Arc::new(cfg),
            client,
            stats,
        }))
    }

    pub fn audit(&self, entry: Value) {
        let Some(path) = &self.cfg.audit_log else {
            return;
        };
        let mut line = Map::new();
        line.insert(
            "ts".into(),
            json!(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
        );
        if self.cfg.test_mode {
            line.insert("test_mode".into(), json!(true));
        }
        if let Value::Object(o) = entry {
            line.extend(o);
        }
        let text = format!("{}\n", Value::Object(line));
        let result = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .and_then(|mut f| f.write_all(text.as_bytes()));
        if let Err(e) = result {
            crate::log::error(&format!("could not write the audit log: {e}"));
        }
    }
}
