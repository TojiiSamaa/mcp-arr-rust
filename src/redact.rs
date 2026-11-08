//! Les secrets que renvoient les *arr (clés d'API d'indexeurs et de clients, mots de passe, jetons, webhooks)
//! n'ont rien à faire dans le contexte d'un agent. Remplacés par défaut ; ARR_REDACT_SECRETS=false pour tout voir.
//! Trois formes : une clé secrète (`"apiKey": "…"`), un champ de réglage (`{"name": "apiKey", "value": "…"}`)
//! et une erreur de validation (`{"propertyName": "ApiKey", "attemptedValue": "…"}`).
//!
//! Règle (nom comparé en minuscules, sans `_`, `-`, `.` ni espace) : secret s'il CONTIENT `token`, `secret`,
//! `password`, `passwd`, `passphrase`, `passkey`, `webhook`, `apikey`, `credential` ; ou s'il vaut `key` ou `psk` ;
//! ou s'il FINIT par le mot `Key` (`consumerKey`, `private_key`), sauf les identifiants connus (`ratingKey`, `sortKey`).
//! `keyword` ou `monkey` ne sont donc pas masqués (`key` n'y est pas un mot final).

use serde_json::Value;

const SECRET_PARTS: [&str; 9] = [
    "token",
    "secret",
    "password",
    "passwd",
    "passphrase",
    "passkey",
    "webhook",
    "apikey",
    "credential",
];
/// Noms finissant par `Key` qui sont des identifiants (Plex, Jellyfin), pas des secrets.
const NOT_SECRET_KEYS: [&str; 4] = [
    "ratingkey",
    "parentratingkey",
    "grandparentratingkey",
    "sortkey",
];

pub const MASK: &str = "[redacted]";

pub fn is_secret(name: &str) -> bool {
    let norm: String = name
        .chars()
        .filter(|c| !matches!(c, '_' | '-' | '.' | ' '))
        .collect::<String>()
        .to_ascii_lowercase();
    if SECRET_PARTS.iter().any(|p| norm.contains(p)) || norm == "key" || norm == "psk" {
        return true;
    }
    // `Key` comme mot final : `fooKey`, `foo_key`, `foo-key`, `FOO_KEY` ; pas `keyword` ni `monkey`.
    let word_key = name.len() > 3 && name.is_char_boundary(name.len() - 3) && {
        let (stem, tail) = name.split_at(name.len() - 3);
        let before = stem.chars().last().unwrap_or(' ');
        (tail == "Key" && before.is_ascii_alphanumeric())
            || (tail.eq_ignore_ascii_case("key") && matches!(before, '_' | '-' | '.'))
    };
    word_key && !NOT_SECRET_KEYS.contains(&norm.as_str())
}

/// Contient-il la valeur de masque ? (un objet lu puis recopié tel quel dans une écriture)
pub fn contains_mask(value: &Value) -> bool {
    match value {
        Value::String(s) => s == MASK,
        Value::Array(a) => a.iter().any(contains_mask),
        Value::Object(o) => o.values().any(contains_mask),
        _ => false,
    }
}

pub fn redact(value: &mut Value) {
    match value {
        Value::Object(map) => {
            // Champ de réglage *arr ou erreur de validation : le nom est secret, la valeur aussi.
            let field_secret = ["name", "propertyName"]
                .iter()
                .any(|k| map.get(*k).and_then(Value::as_str).is_some_and(is_secret));
            for (k, v) in map.iter_mut() {
                let secret = is_secret(k)
                    || (field_secret && matches!(k.as_str(), "value" | "attemptedValue"));
                if secret && matches!(v, Value::String(s) if !s.is_empty()) {
                    *v = Value::String(MASK.into());
                } else {
                    redact(v);
                }
            }
        }
        Value::Array(a) => a.iter_mut().for_each(redact),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn masks_nested_secrets() {
        let mut v = json!({"records": [{"name": "idx", "apiKey": "abc", "ok": {"password": "z", "empty": ""}}]});
        redact(&mut v);
        assert_eq!(v["records"][0]["apiKey"], MASK);
        assert_eq!(v["records"][0]["ok"]["password"], MASK);
        assert_eq!(v["records"][0]["ok"]["empty"], "");
        assert_eq!(v["records"][0]["name"], "idx");
    }

    #[test]
    fn masks_setting_fields() {
        let mut v = json!({"fields": [{"name": "apiKey", "value": "k"}, {"name": "baseUrl", "value": "u"}]});
        redact(&mut v);
        assert_eq!(v["fields"][0]["value"], MASK);
        assert_eq!(v["fields"][1]["value"], "u");
    }

    /// Règle par sous-chaîne, insensible à la casse ; `key` seulement comme mot final.
    #[test]
    fn substring_rule_and_false_positives() {
        let secret = [
            "webHookUrl",
            "botToken",
            "authToken",
            "accessToken",
            "proxyPassword",
            "sslCertPassword",
            "passwordConfirmation",
            "PASSWD",
            "rpc_secret",
            "consumerKey",
            "private_key",
            "key",
            "ApiKey",
            "x-api-key",
            "torrentPasskey",
        ];
        let kept = [
            "keyword",
            "keywords",
            "monkey",
            "ratingKey",
            "sortKey",
            "username",
            "baseUrl",
            "apiPath",
        ];
        for name in secret {
            let mut v = json!({ name: "s", "fields": [{ "name": name, "value": "s" }], "errors": [{ "propertyName": name, "attemptedValue": "s" }] });
            redact(&mut v);
            assert!(!v.to_string().contains("\"s\""), "{name} non masqué : {v}");
        }
        for name in kept {
            let mut v = json!({ name: "k", "fields": [{ "name": name, "value": "k" }] });
            redact(&mut v);
            assert_eq!(v[name], "k", "{name} masqué à tort");
            assert_eq!(v["fields"][0]["value"], "k", "{name} masqué à tort");
        }
    }
}
