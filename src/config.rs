//! Configuration : variables d'environnement (mêmes noms que mcp-arr pour les services, `ARR_*` pour la garde).
//! Priorité : options de ligne de commande > environnement > fichier `.env`. Tout est fermé par défaut.
//! Le mode test ne s'active qu'ici, au démarrage (`--test` ou `ARR_TEST_MODE=1`) : aucun outil ne peut le changer.

use crate::error::{ArrError, ErrorType, Result};
use crate::services::SERVICES;
use std::collections::HashMap;

pub type Env = HashMap<String, String>;

/// Quand demander une confirmation humaine (si le client sait demander).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Elicit {
    Off,
    Destructive,
    All,
}

/// Réponse simulée de l'humain en mode test, à la place de l'elicitation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Approver {
    Approve,
    Deny,
    Timeout,
    Absent,
}

impl Approver {
    pub fn as_str(self) -> &'static str {
        match self {
            Approver::Approve => "approve",
            Approver::Deny => "deny",
            Approver::Timeout => "timeout",
            Approver::Absent => "absent",
        }
    }
}

/// Un service configuré (URL et clé présentes ; en mode test, ni l'une ni l'autre n'est lue).
#[derive(Clone)]
pub struct ServiceConf {
    pub name: &'static str,
    pub url: String,
    pub key: String,
}

impl std::fmt::Debug for ServiceConf {
    // La clé n'apparaît jamais dans un Debug (journaux, paniques).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServiceConf")
            .field("name", &self.name)
            .field("url", &self.url)
            .field("key", &"[redacted]")
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    pub services: Vec<ServiceConf>,
    pub verify_ssl: bool,
    pub timeout_secs: u64,
    pub allow_writes: bool,
    pub allow_delete: bool,
    pub dry_run: bool,
    pub elicit: Elicit,
    /// Secondes d'attente d'une réponse humaine à une elicitation (ARR_ELICIT_TIMEOUT, défaut 120) ; au-delà : refus.
    pub elicit_timeout_secs: u64,
    /// Repli opt-in : un client sans elicitation peut confirmer avec `confirm: true`.
    pub confirm_arg: bool,
    /// Services écrivables ; vide = tous les services configurés (les interrupteurs restent la vraie fermeture).
    pub write_services: Vec<String>,
    pub max_limit: i64,
    pub max_response_chars: usize,
    pub redact: bool,
    pub audit_log: Option<String>,
    pub mcp_token: Option<String>,
    pub allowed_hosts: Vec<String>,
    pub cors_origins: Vec<String>,
    pub log_level: String,
    pub test_mode: bool,
    pub test_approver: Approver,
    pub stats_file: Option<String>,
}

impl Config {
    pub fn service(&self, name: &str) -> Option<&ServiceConf> {
        self.services.iter().find(|s| s.name == name)
    }
    pub fn configured(&self) -> Vec<&'static str> {
        self.services.iter().map(|s| s.name).collect()
    }
}

pub fn process_env() -> Env {
    std::env::vars().collect()
}

/// Lit un `.env` simple (`CLE=valeur`, `#` commentaire, guillemets facultatifs). Ne remplace rien d'existant.
pub fn merge_dotenv(env: &mut Env, text: &str) {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let v = v.trim().trim_matches(|c| c == '"' || c == '\'');
        env.entry(k.trim().trim_start_matches("export ").to_string())
            .or_insert_with(|| v.to_string());
    }
}

fn flag(env: &Env, key: &str, default: bool) -> Result<bool> {
    match env.get(key).map(|v| v.trim().to_ascii_lowercase()) {
        None => Ok(default),
        Some(v) if v.is_empty() => Ok(default),
        Some(v) if ["1", "true", "yes", "on"].contains(&v.as_str()) => Ok(true),
        Some(v) if ["0", "false", "no", "off"].contains(&v.as_str()) => Ok(false),
        Some(v) => Err(ArrError::new(
            ErrorType::ConfigError,
            format!("{key}: \"{v}\" is not a boolean (true/false)."),
        )),
    }
}

fn number<T: std::str::FromStr>(env: &Env, key: &str, default: T) -> Result<T> {
    match env.get(key).map(|v| v.trim()).filter(|v| !v.is_empty()) {
        None => Ok(default),
        Some(v) => v.parse().map_err(|_| {
            ArrError::new(
                ErrorType::ConfigError,
                format!("{key}: \"{v}\" is not a valid number."),
            )
        }),
    }
}

fn list(env: &Env, key: &str) -> Vec<String> {
    env.get(key)
        .map(|v| {
            v.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn text(env: &Env, key: &str) -> Option<String> {
    env.get(key)
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn read_services(env: &Env, test_mode: bool) -> Result<Vec<ServiceConf>> {
    let mut out = Vec::new();
    for def in &SERVICES {
        if test_mode {
            // Aucune URL ni clé lue : tout passe par le mock, l'adresse n'est qu'une étiquette.
            out.push(ServiceConf {
                name: def.name,
                url: format!("test://{}", def.name),
                key: String::new(),
            });
            continue;
        }
        let (Some(url), Some(key)) = (text(env, def.url_env), text(env, def.key_env)) else {
            continue;
        };
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err(ArrError::new(
                ErrorType::ConfigError,
                format!(
                    "{}: \"{url}\" must start with http:// or https://.",
                    def.url_env
                ),
            ));
        }
        out.push(ServiceConf {
            name: def.name,
            url: url.trim_end_matches('/').to_string(),
            key,
        });
    }
    Ok(out)
}

pub fn load_config(env: &Env) -> Result<Config> {
    let test_mode = flag(env, "ARR_TEST_MODE", false)?;
    let write_services = list(env, "ARR_WRITE_SERVICES");
    if let Some(bad) = write_services
        .iter()
        .find(|s| crate::services::get(s).is_none())
    {
        return Err(ArrError::new(
            ErrorType::ConfigError,
            format!("ARR_WRITE_SERVICES: unknown service \"{bad}\"."),
        )
        .hint(format!("Known: {}.", crate::services::names().join(", "))));
    }
    Ok(Config {
        services: read_services(env, test_mode)?,
        verify_ssl: flag(env, "VERIFY_SSL", true)?,
        timeout_secs: number(env, "ARR_TIMEOUT", 30)?,
        allow_writes: flag(env, "ARR_ALLOW_WRITES", false)?,
        allow_delete: flag(env, "ARR_ALLOW_DELETE", false)?,
        dry_run: flag(env, "ARR_DRY_RUN", false)?,
        confirm_arg: flag(env, "ARR_CONFIRM_ARG", false)?,
        elicit: match text(env, "ARR_ELICIT")
            .map(|v| v.to_ascii_lowercase())
            .as_deref()
        {
            None | Some("true") | Some("destructive") => Elicit::Destructive,
            Some("false") | Some("off") | Some("0") => Elicit::Off,
            Some("all") => Elicit::All,
            Some(v) => {
                return Err(ArrError::new(
                    ErrorType::ConfigError,
                    format!("ARR_ELICIT: \"{v}\" is not valid (off, destructive, all)."),
                ))
            }
        },
        elicit_timeout_secs: number(env, "ARR_ELICIT_TIMEOUT", 120)?,
        write_services,
        max_limit: number(env, "ARR_MAX_LIMIT", 100)?,
        max_response_chars: number(env, "ARR_MAX_RESPONSE_CHARS", 60_000)?,
        redact: flag(env, "ARR_REDACT_SECRETS", true)?,
        audit_log: text(env, "ARR_AUDIT_LOG"),
        mcp_token: text(env, "ARR_MCP_TOKEN"),
        allowed_hosts: list(env, "ARR_MCP_ALLOWED_HOSTS"),
        cors_origins: list(env, "CORS_ORIGINS"),
        log_level: text(env, "LOG_LEVEL")
            .unwrap_or_else(|| "INFO".into())
            .to_ascii_uppercase(),
        test_mode,
        test_approver: match text(env, "ARR_TEST_APPROVER")
            .map(|v| v.to_ascii_lowercase())
            .as_deref()
        {
            None | Some("deny") => Approver::Deny,
            Some("approve") => Approver::Approve,
            Some("timeout") => Approver::Timeout,
            Some("absent") => Approver::Absent,
            Some(v) => {
                return Err(ArrError::new(
                    ErrorType::ConfigError,
                    format!(
                        "ARR_TEST_APPROVER: \"{v}\" is not valid (approve, deny, timeout, absent)."
                    ),
                ))
            }
        },
        stats_file: text(env, "ARR_STATS_FILE"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> Env {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn defaults_are_closed() {
        let c = load_config(&env(&[])).unwrap();
        assert!(c.services.is_empty());
        assert!(!c.allow_writes && !c.allow_delete && !c.dry_run && !c.confirm_arg && !c.test_mode);
        assert!(c.redact && c.verify_ssl && c.mcp_token.is_none());
        assert_eq!(c.elicit, Elicit::Destructive);
        assert_eq!(c.elicit_timeout_secs, 120);
        assert_eq!(c.test_approver, Approver::Deny);
    }

    #[test]
    fn service_needs_url_and_key() {
        let c = load_config(&env(&[
            ("SONARR_URL", "http://sonarr.example:8989/"),
            ("RADARR_URL", "http://r.example"),
        ]))
        .unwrap();
        assert!(c.services.is_empty());
        let c = load_config(&env(&[
            ("SONARR_URL", "http://sonarr.example:8989/"),
            ("SONARR_API_KEY", "k"),
        ]))
        .unwrap();
        assert_eq!(c.configured(), vec!["sonarr"]);
        assert_eq!(
            c.service("sonarr").unwrap().url,
            "http://sonarr.example:8989"
        );
        assert!(!format!("{c:?}").contains("\"k\""));
        assert!(load_config(&env(&[("SONARR_URL", "sonarr"), ("SONARR_API_KEY", "k")])).is_err());
    }

    #[test]
    fn test_mode_reads_no_url_or_key() {
        let c = load_config(&env(&[
            ("ARR_TEST_MODE", "1"),
            ("SONARR_URL", "http://real.example"),
            ("SONARR_API_KEY", "real"),
        ]))
        .unwrap();
        assert!(c.test_mode);
        assert_eq!(c.services.len(), SERVICES.len());
        assert!(c
            .services
            .iter()
            .all(|s| s.key.is_empty() && s.url.starts_with("test://")));
    }

    #[test]
    fn bad_values_are_refused() {
        assert!(load_config(&env(&[("ARR_ALLOW_WRITES", "maybe")])).is_err());
        assert!(load_config(&env(&[("ARR_TEST_APPROVER", "yes")])).is_err());
        assert!(load_config(&env(&[("ARR_WRITE_SERVICES", "sonarr,plex")])).is_err());
        assert!(load_config(&env(&[("ARR_ELICIT", "sometimes")])).is_err());
    }

    #[test]
    fn dotenv_never_overrides() {
        let mut e = env(&[("A", "1")]);
        merge_dotenv(&mut e, "# c\nA=2\nexport B=\"x y\"\n");
        assert_eq!(e["A"], "1");
        assert_eq!(e["B"], "x y");
    }
}
