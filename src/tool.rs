//! Définition d'un outil MCP, arguments reçus et contexte d'un appel.

use crate::client::{Client, Method};
use crate::config::Config;
use crate::error::Result;
use crate::runtime::Runtime;
use crate::schema::Schema;
use serde_json::{json, Map, Value};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

pub type BoxFut<T> = Pin<Box<dyn Future<Output = T> + Send>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
    Destructive,
}

impl Access {
    pub fn as_str(self) -> &'static str {
        match self {
            Access::Read => "read",
            Access::Write => "write",
            Access::Destructive => "destructive",
        }
    }
}

/// Arguments déjà validés par le schéma de l'outil : les champs obligatoires sont présents et du bon type.
pub struct Args(pub Map<String, Value>);

impl Args {
    pub fn str(&self, k: &str) -> Option<&str> {
        self.0.get(k).and_then(Value::as_str)
    }
    pub fn text(&self, k: &str) -> &str {
        self.str(k).unwrap_or("")
    }
    pub fn int(&self, k: &str) -> Option<i64> {
        self.0
            .get(k)
            .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
    }
    pub fn flag(&self, k: &str) -> bool {
        self.0.get(k).and_then(Value::as_bool).unwrap_or(false)
    }
    pub fn obj(&self, k: &str) -> Map<String, Value> {
        self.0
            .get(k)
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default()
    }
    pub fn list(&self, k: &str) -> Vec<Value> {
        self.0
            .get(k)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    }
    pub fn strings(&self, k: &str) -> Vec<String> {
        self.list(k)
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect()
    }
    /// « chaîne ou liste de chaînes ».
    pub fn string_or_list(&self, k: &str) -> Vec<String> {
        match self.0.get(k) {
            Some(Value::String(s)) => vec![s.clone()],
            Some(Value::Array(a)) => a
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect(),
            _ => vec![],
        }
    }
    pub fn to_value(&self) -> Value {
        Value::Object(self.0.clone())
    }
}

pub struct Ctx {
    pub client: Client,
    pub cfg: Arc<Config>,
    pub runtime: Arc<Runtime>,
    pub dry_run: bool,
    pub dry_log: Mutex<Vec<Value>>,
    pub writes: Mutex<Vec<Value>>,
}

impl Ctx {
    /// UNIQUE chemin d'écriture. En simulation on note la requête et on n'envoie rien (renvoie `Null`) ;
    /// sinon on revérifie la politique (défense en profondeur), on envoie et on consigne pour l'audit.
    pub async fn write(
        &self,
        service: &str,
        method: Method,
        path: &str,
        query: &[(String, String)],
        body: Option<&Value>,
    ) -> Result<Value> {
        let full = crate::client::full_path(service, path)?;
        crate::policy::check_service_writable(&self.cfg, service)?;
        if self.dry_run {
            self.dry_log.lock().unwrap().push(json!({ "service": service, "method": method.as_str(), "path": full, "query": query, "body": body }));
            return Ok(Value::Null);
        }
        // Destructif : toute suppression, et la coupure d'un flux Tracearr (un POST).
        let destructive = method == Method::DELETE || full.ends_with("/terminate");
        crate::policy::check_writes_on(&self.cfg, destructive)?;
        let result = self
            .client
            .request(service, method.clone(), path, query, body)
            .await;
        let outcome = match &result {
            Ok(_) => json!("ok"),
            Err(e) => json!(e.kind.as_str()),
        };
        self.writes.lock().unwrap().push(json!({ "service": service, "method": method.as_str(), "path": full, "outcome": outcome }));
        result
    }
}

pub type Handler = fn(Args, Arc<Ctx>) -> BoxFut<Result<Value>>;
/// Contrôle de portée avant de déranger un humain (ressource interdite, nom de commande hors liste...).
pub type Precheck = fn(&Config, &Args) -> Result<()>;
pub type Confirmation = fn(&Args) -> String;

pub struct ToolDef {
    pub name: &'static str,
    pub title: &'static str,
    pub description: String,
    pub input: Schema,
    pub access: Access,
    pub idempotent: bool,
    pub open_world: bool,
    pub handler: Handler,
    pub precheck: Option<Precheck>,
    pub confirmation: Option<Confirmation>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{load_config, Env};
    use crate::error::ErrorType;

    /// Le second contrôle de `Ctx::write` traite la coupure d'un flux comme destructive.
    #[tokio::test]
    async fn terminate_post_needs_allow_delete_in_ctx_write() {
        let e: Env = [
            ("TRACEARR_URL", "http://t.example"),
            ("TRACEARR_API_KEY", "k"),
            ("ARR_ALLOW_WRITES", "true"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let calls = Arc::new(Mutex::new(0));
        let c2 = calls.clone();
        let mock: crate::client::MockFn = Arc::new(move |_, _, _, _, _| {
            *c2.lock().unwrap() += 1;
            Ok(json!({}))
        });
        let rt = Runtime::with_mock(load_config(&e).unwrap(), Some(mock)).unwrap();
        let ctx = Ctx {
            client: rt.client.clone(),
            cfg: rt.cfg.clone(),
            runtime: rt.clone(),
            dry_run: false,
            dry_log: Mutex::new(vec![]),
            writes: Mutex::new(vec![]),
        };
        let err = ctx
            .write("tracearr", Method::POST, "/streams/s1/terminate", &[], None)
            .await
            .unwrap_err();
        assert!(err.is(ErrorType::DeleteDisabled), "{err}");
        assert_eq!(*calls.lock().unwrap(), 0);
    }
}
