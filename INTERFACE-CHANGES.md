# Écarts et précisions d'interface (noyau, agent A)

- `Method` : utiliser `crate::client::Method` (réexport de reqwest) ; `client.rs` est le seul fichier qui cite `reqwest`.
- `Precheck` = `fn(&Config, &Args) -> Result<()>` (plus de registre de types).
- `MockFn` reçoit le chemin COMPLET avec préfixe (`/api/v3/series/1`, `/api/v1/public/stats` pour Tracearr).
- Objet absent = `ErrorType::NotFound` AVEC `details` (sinon le serveur le traite comme un chemin inconnu, pas `found:false`).
  Aide : `crate::client::absent(service, full_path)`.
- Nouveaux `ErrorType` : `ServiceNotAllowed`, `ResourceNotAllowed`, `NotConfigured`, `TestModeEgressBlocked` (`"TEST_MODE_EGRESS_BLOCKED"`).
- `crate::client::{api_path, full_path, query_pairs}` ; `crate::services::{get, names, SERVICES}`, `ServiceDef::{resource, resource_names, prefix_for, is_arr}`.
- `Config::service(name) -> Option<&ServiceConf>`, `Config::configured() -> Vec<&str>` ; en mode test tous les services sont « configurés » (url `test://<nom>`, clé vide).
- `ARR_WRITE_SERVICES` vide = tous les services (les interrupteurs ARR_ALLOW_WRITES/ARR_ALLOW_DELETE restent la fermeture).
- Le serveur refuse AVANT l'outil : service connu mais non configuré (`not_configured`), service hors liste blanche pour un outil d'écriture.
  `Ctx::write` revérifie liste blanche + interrupteurs (DELETE et POST `…/terminate` = second interrupteur).
- `redact::redact(&mut v)` (plus de paramètre `tokens`), `redact::is_secret`, `redact::contains_mask` ; le serveur masque aussi `would_send` et `details`.
- `Session::abandon_pending()` : entrée stdio fermée, les confirmations en attente échouent tout de suite.
- Les réponses sont masquées (`redact`) par le serveur, pas besoin de le faire dans les outils.
- Tests : `Runtime::with_mock(cfg, Option<MockFn>)` ; `server::all_tools(cfg)` ; `http::bind(registry, opts) -> (port, future)` pour un port 0.
- HTTP : port par défaut 3000 (comme mcp-arr), `MCP_PATH` défaut `/mcp`, `/health` sans jeton = `{"status":"ok"}`.
- `rust-version` passé de 1.80 à 1.82 (`Option::is_none_or` utilisé dans tools_write.rs).
- POUR C : avec rust-version 1.82 (requis aussi par tests/proof_ab.rs, `is_none_or`), clippy demande
  `only.is_none_or(|o| o.contains(&resource))` à la place de `only.map_or(true, …)` (src/tools_write.rs:72).
- DÉFINITIF : `Session::new(registry, out)` à DEUX arguments (pas de jeton client/passthrough). POUR E : tests/common/mod.rs:111 doit retirer le 3e `None`.
- POUR E : tests/e2e.rs:287 cherche la sous-chaîne "arr_add" dans tools/list ; la description de arr_lookup (B) la contient (« ids arr_add needs ») : comparer les NOMS des outils, pas le texte.
