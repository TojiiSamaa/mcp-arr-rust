//! Qui peut écrire quoi, vérifié AVANT toute requête : interrupteurs (ARR_ALLOW_WRITES, puis ARR_ALLOW_DELETE)
//! et liste blanche de services (ARR_WRITE_SERVICES). Le dry-run passe les interrupteurs : il n'envoie rien.

use crate::config::Config;
use crate::error::{ArrError, ErrorType, Result};

pub fn check_writes_on(cfg: &Config, is_delete: bool) -> Result<()> {
    if !cfg.allow_writes && !cfg.dry_run {
        return Err(ArrError::new(ErrorType::WriteDisabled, "This server is read-only: nothing was sent.")
            .hint("Do not retry. The operator can enable writes with ARR_ALLOW_WRITES=true, or preview with ARR_DRY_RUN=true."));
    }
    if is_delete && !cfg.allow_delete && !cfg.dry_run {
        return Err(ArrError::new(ErrorType::DeleteDisabled, "Deletions and other destructive actions are disabled: nothing was sent.")
            .hint("Do not retry. The operator can enable them with ARR_ALLOW_DELETE=true (on top of ARR_ALLOW_WRITES)."));
    }
    Ok(())
}

/// Le service est-il écrivable ? Liste vide = tous les services ; sinon seulement ceux nommés.
pub fn check_service_writable(cfg: &Config, service: &str) -> Result<()> {
    if cfg.write_services.is_empty() || cfg.write_services.iter().any(|s| s == service) {
        return Ok(());
    }
    Err(ArrError::new(ErrorType::ServiceNotAllowed, format!("Writing to {service} is not allowed: it is not in ARR_WRITE_SERVICES. Nothing was sent."))
        .hint("Do not retry or look for another route. The operator can add it to ARR_WRITE_SERVICES."))
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
    fn switches() {
        assert!(check_writes_on(&cfg(&[]), false)
            .unwrap_err()
            .is(ErrorType::WriteDisabled));
        assert!(check_writes_on(&cfg(&[("ARR_ALLOW_DELETE", "true")]), true)
            .unwrap_err()
            .is(ErrorType::WriteDisabled));
        assert!(check_writes_on(&cfg(&[("ARR_ALLOW_WRITES", "true")]), false).is_ok());
        assert!(check_writes_on(&cfg(&[("ARR_ALLOW_WRITES", "true")]), true)
            .unwrap_err()
            .is(ErrorType::DeleteDisabled));
        assert!(check_writes_on(
            &cfg(&[("ARR_ALLOW_WRITES", "true"), ("ARR_ALLOW_DELETE", "true")]),
            true
        )
        .is_ok());
        assert!(check_writes_on(&cfg(&[("ARR_DRY_RUN", "true")]), true).is_ok());
    }

    #[test]
    fn service_whitelist() {
        assert!(check_service_writable(&cfg(&[]), "sonarr").is_ok());
        let c = cfg(&[("ARR_WRITE_SERVICES", "radarr, prowlarr")]);
        assert!(check_service_writable(&c, "radarr").is_ok());
        assert!(check_service_writable(&c, "sonarr")
            .unwrap_err()
            .is(ErrorType::ServiceNotAllowed));
    }
}
