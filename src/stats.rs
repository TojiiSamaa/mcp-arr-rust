//! Compteurs du serveur (partagés via `Runtime`) et outil `arr_stats` : appels par outil, erreurs, latences
//! p50/p95, dernier appel, appels réels vs mode test, coupes réseau, réponses de l'approbateur.
//! Export JSON après chaque appel si `ARR_STATS_FILE` est réglé.

use crate::config::Approver;
use crate::error::Result;
use crate::schema::Schema;
use crate::tool::{Access, Args, BoxFut, Ctx, ToolDef};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

/// Fenêtre glissante des latences gardées par outil.
const WINDOW: usize = 1000;

#[derive(Default)]
struct ToolStats {
    calls: u64,
    errors: u64,
    latencies_ms: VecDeque<u64>,
    last_call: Option<String>,
    last_error: Option<String>,
}

#[derive(Default)]
struct Inner {
    tools: BTreeMap<String, ToolStats>,
    real_calls: u64,
    test_calls: u64,
    requests_real: u64,
    requests_mock: u64,
    egress_blocked: u64,
    approvals: BTreeMap<&'static str, u64>,
    elicit_timeouts: u64,
}

#[derive(Default)]
pub struct Stats {
    inner: Mutex<Inner>,
}

/// Percentile par rang le plus proche sur une liste triée (`p` entre 0 et 100).
pub fn percentile(sorted: &[u64], p: f64) -> Option<u64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    Some(sorted[rank.clamp(1, sorted.len()) - 1])
}

impl Stats {
    /// Un appel d'outil terminé (`error` = type d'erreur stable).
    pub fn record_call(&self, tool: &str, error: Option<&str>, ms: u64, test_mode: bool) {
        let mut g = self.inner.lock().unwrap();
        if test_mode {
            g.test_calls += 1;
        } else {
            g.real_calls += 1;
        }
        let t = g.tools.entry(tool.to_string()).or_default();
        t.calls += 1;
        if let Some(e) = error {
            t.errors += 1;
            t.last_error = Some(e.to_string());
        }
        if t.latencies_ms.len() == WINDOW {
            t.latencies_ms.pop_front();
        }
        t.latencies_ms.push_back(ms);
        t.last_call = Some(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
    }

    /// Une requête partie vers un service (`real`) ou servie par le mock.
    pub fn request(&self, real: bool) {
        let mut g = self.inner.lock().unwrap();
        if real {
            g.requests_real += 1;
        } else {
            g.requests_mock += 1;
        }
    }

    pub fn egress_blocked(&self) {
        self.inner.lock().unwrap().egress_blocked += 1;
    }

    /// Une elicitation restée sans réponse humaine dans ARR_ELICIT_TIMEOUT.
    pub fn elicit_timeout(&self) {
        self.inner.lock().unwrap().elicit_timeouts += 1;
    }

    /// Réponse de l'approbateur simulé (mode test).
    pub fn approval(&self, a: Approver) {
        *self
            .inner
            .lock()
            .unwrap()
            .approvals
            .entry(a.as_str())
            .or_default() += 1;
    }

    pub fn snapshot(&self) -> Value {
        let g = self.inner.lock().unwrap();
        let mut tools = Map::new();
        for (name, t) in &g.tools {
            let mut sorted: Vec<u64> = t.latencies_ms.iter().copied().collect();
            sorted.sort_unstable();
            tools.insert(
                name.clone(),
                json!({
                    "calls": t.calls,
                    "errors": t.errors,
                    "p50_ms": percentile(&sorted, 50.0),
                    "p95_ms": percentile(&sorted, 95.0),
                    "last_call": t.last_call,
                    "last_error": t.last_error,
                }),
            );
        }
        let ap = |k: &str| g.approvals.get(k).copied().unwrap_or(0);
        json!({
            "tools": tools,
            "calls": { "real": g.real_calls, "test": g.test_calls },
            "network": { "requests_real": g.requests_real, "requests_mock": g.requests_mock, "egress_blocked": g.egress_blocked },
            "approver": { "approved": ap("approve"), "denied": ap("deny"), "timeouts": ap("timeout"), "absent": ap("absent") },
            "elicitation": { "timeouts": g.elicit_timeouts },
        })
    }

    /// Écrit le résumé dans `path` (fichier temporaire puis renommage : jamais de JSON à moitié écrit).
    pub fn export(&self, path: &str) {
        let tmp = format!("{path}.tmp");
        let text = serde_json::to_string_pretty(&self.snapshot()).unwrap_or_default();
        if let Err(e) = std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, path)) {
            crate::log::warn(&format!("could not write ARR_STATS_FILE: {e}"));
        }
    }
}

fn run(_: Args, ctx: Arc<Ctx>) -> BoxFut<Result<Value>> {
    Box::pin(async move {
        let mut v = ctx.runtime.stats.snapshot();
        v["test_mode"] = json!(ctx.cfg.test_mode);
        Ok(v)
    })
}

pub fn tool() -> ToolDef {
    ToolDef {
        name: "arr_stats",
        title: "Server statistics",
        description: "Usage statistics of this MCP server since it started: calls and errors per tool, latency p50/p95, last call, real vs test-mode calls, blocked network egress, test approver answers. Takes no `service`; sends nothing to any service.".into(),
        input: Schema::new(vec![]),
        access: Access::Read,
        idempotent: true,
        open_world: false,
        handler: run,
        precheck: None,
        confirmation: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_nearest_rank() {
        let v: Vec<u64> = (1..=100).collect();
        assert_eq!(percentile(&v, 50.0), Some(50));
        assert_eq!(percentile(&v, 95.0), Some(95));
        assert_eq!(percentile(&[7], 95.0), Some(7));
        assert_eq!(percentile(&[1, 2], 50.0), Some(1));
        assert_eq!(percentile(&[], 50.0), None);
    }

    #[test]
    fn counts_and_exports() {
        let s = Stats::default();
        for ms in [10, 20, 30] {
            s.record_call("arr_list", None, ms, true);
        }
        s.record_call("arr_list", Some("not_found"), 40, false);
        s.egress_blocked();
        s.approval(Approver::Deny);
        let v = s.snapshot();
        assert_eq!(v["tools"]["arr_list"]["calls"], 4);
        assert_eq!(v["tools"]["arr_list"]["errors"], 1);
        assert_eq!(v["tools"]["arr_list"]["p50_ms"], 20);
        assert_eq!(v["tools"]["arr_list"]["p95_ms"], 40);
        assert_eq!(v["calls"]["test"], 3);
        assert_eq!(v["network"]["egress_blocked"], 1);
        assert_eq!(v["approver"]["denied"], 1);
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("stats.json");
        s.export(p.to_str().unwrap());
        let back: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(back["tools"]["arr_list"]["calls"], 4);
    }
}
