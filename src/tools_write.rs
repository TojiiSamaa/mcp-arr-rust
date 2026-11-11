//! Outils d'écriture et destructifs (14 à 22). Toute écriture réseau passe par `Ctx::write` (qui seul sait
//! ce qu'est un dry-run) ; les lectures préalables par `ctx.client.get`. Les valeurs inconnues ou ambiguës
//! sont refusées AVANT toute requête.

use crate::client::Method;
use crate::config::Config;
use crate::error::{ArrError, ErrorType, Result};
use crate::schema::{
    any, array_of, boolean, enumeration, integer, string, unknown_record, Field, Schema, Ty,
};
use crate::tool::{Access, Args, BoxFut, Confirmation, Ctx, Handler, Precheck, ToolDef};
use serde_json::{json, Map, Value};
use std::sync::Arc;

type Query = Vec<(String, String)>;

const ARR3: [&str; 3] = ["sonarr", "radarr", "lidarr"];
const ARR4: [&str; 4] = ["sonarr", "radarr", "lidarr", "prowlarr"];

/// Ressources écrivables : (nom, services qui la portent). Liste blanche unique pour update/delete/create/test.
const RESOURCES: [(&str, &[&str]); 8] = [
    ("series", &["sonarr"]),
    ("movie", &["radarr"]),
    ("artist", &["lidarr"]),
    ("album", &["lidarr"]),
    ("indexer", &ARR4),
    ("downloadclient", &ARR4),
    ("tag", &ARR4),
    ("qualityprofile", &ARR3),
];
const CREATABLE: [&str; 4] = ["tag", "qualityprofile", "indexer", "downloadclient"];
const TESTABLE: [&str; 2] = ["indexer", "downloadclient"];

/// Commandes autorisées : (service, nom, paramètres (nom, liste d'entiers ?, obligatoire ?)).
type Param = (&'static str, bool, bool);
const COMMANDS: [(&str, &str, &[Param]); 17] = [
    ("sonarr", "RefreshSeries", &[("seriesId", false, false)]),
    ("sonarr", "RescanSeries", &[("seriesId", false, false)]),
    ("sonarr", "SeriesSearch", &[("seriesId", false, true)]),
    ("sonarr", "EpisodeSearch", &[("episodeIds", true, true)]),
    (
        "sonarr",
        "SeasonSearch",
        &[("seriesId", false, true), ("seasonNumber", false, true)],
    ),
    ("sonarr", "MissingEpisodeSearch", &[]),
    ("sonarr", "RssSync", &[]),
    ("radarr", "RefreshMovie", &[("movieIds", true, false)]),
    ("radarr", "RescanMovie", &[("movieId", false, false)]),
    ("radarr", "MoviesSearch", &[("movieIds", true, true)]),
    ("radarr", "MissingMoviesSearch", &[]),
    ("radarr", "RssSync", &[]),
    ("lidarr", "RefreshArtist", &[("artistId", false, false)]),
    ("lidarr", "RescanArtist", &[("artistId", false, false)]),
    ("lidarr", "ArtistSearch", &[("artistId", false, true)]),
    ("lidarr", "AlbumSearch", &[("albumIds", true, true)]),
    ("lidarr", "MissingAlbumSearch", &[]),
];

fn bad(msg: impl Into<String>) -> ArrError {
    ArrError::new(ErrorType::InvalidInput, msg)
}

fn boxed<F: std::future::Future<Output = Result<Value>> + Send + 'static>(
    f: F,
) -> BoxFut<Result<Value>> {
    Box::pin(f)
}

fn check_service(service: &str, allowed: &[&str]) -> Result<()> {
    if allowed.contains(&service) {
        Ok(())
    } else {
        Err(bad(format!(
            "`service` must be one of: {}.",
            allowed.join(", ")
        )))
    }
}

/// Ressource connue ET portée par ce service, sinon refus.
fn check_resource(service: &str, resource: &str, only: Option<&[&str]>) -> Result<()> {
    let ok = RESOURCES
        .iter()
        .any(|(r, s)| *r == resource && s.contains(&service))
        && only.is_none_or(|o| o.contains(&resource));
    if ok {
        Ok(())
    } else {
        Err(bad(format!(
            "Resource `{resource}` is not writable on `{service}`."
        )))
    }
}

fn positive(a: &Args, k: &str) -> Result<i64> {
    a.int(k)
        .filter(|i| *i >= 1)
        .ok_or_else(|| bad(format!("`{k}` must be a positive integer.")))
}

fn ints_of(v: &Value, k: &str) -> Result<Vec<i64>> {
    let items = v
        .as_array()
        .filter(|l| !l.is_empty() && l.len() <= 500)
        .ok_or_else(|| {
            bad(format!(
                "`{k}` must be a list of 1 to 500 positive integers."
            ))
        })?;
    items
        .iter()
        .map(|i| i.as_i64().filter(|n| *n >= 1))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| bad(format!("`{k}` must hold positive integers only.")))
}

/// Ce que verrait le serveur distant : sert d'aperçu en dry-run (la garantie « rien n'est envoyé » est celle de `Ctx::write`).
fn would(
    service: &str,
    method: &str,
    path: &str,
    query: &[(String, String)],
    body: Option<&Value>,
) -> Value {
    json!({ "service": service, "method": method, "path": path, "query": query, "body": body })
}

// ---------- arr_add ----------

fn add_body(service: &str, a: &Args) -> Result<Value> {
    check_service(service, &ARR3)?;
    let (id_field, flags): (&str, &[&str]) = match service {
        "sonarr" => (
            "tvdbId",
            &["searchForMissingEpisodes", "searchForCutoffUnmetEpisodes"],
        ),
        "radarr" => ("tmdbId", &["searchForMovie"]),
        _ => ("foreignArtistId", &["searchForMissingAlbums"]),
    };
    let mut item = a.obj("item");
    if item
        .get(id_field)
        .is_none_or(|v| v.is_null() || v == &json!(0) || v == &json!(""))
    {
        return Err(bad(format!(
            "`item` must be a lookup result carrying `{id_field}` (use arr_lookup)."
        )));
    }
    let root = a.text("rootFolderPath");
    if root.trim().is_empty() {
        return Err(bad("`rootFolderPath` is required."));
    }
    item.remove("id");
    item.insert(
        "qualityProfileId".into(),
        json!(positive(a, "qualityProfileId")?),
    );
    item.insert("rootFolderPath".into(), json!(root));
    item.insert(
        "monitored".into(),
        json!(a
            .0
            .get("monitored")
            .and_then(Value::as_bool)
            .unwrap_or(true)),
    );
    if service == "lidarr" {
        item.insert(
            "metadataProfileId".into(),
            json!(positive(a, "metadataProfileId")
                .map_err(|_| bad("`metadataProfileId` is required for lidarr."))?),
        );
    }
    if a.0.contains_key("tags") {
        let tags = a.list("tags");
        item.insert(
            "tags".into(),
            json!(if tags.is_empty() {
                vec![]
            } else {
                ints_of(&json!(tags), "tags")?
            }),
        );
    }
    // Recherche automatique : fermée, quoi que porte l'objet de lookup, sauf `search: true` explicite.
    let search = a.flag("search");
    let mut opts: Map<String, Value> = item
        .get("addOptions")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for f in flags {
        opts.insert((*f).into(), json!(false));
    }
    opts.insert(flags[0].into(), json!(search));
    item.insert("addOptions".into(), Value::Object(opts));
    Ok(Value::Object(item))
}

fn add(a: Args, ctx: Arc<Ctx>) -> BoxFut<Result<Value>> {
    boxed(async move {
        let service = a.text("service");
        let body = add_body(service, &a)?;
        let path = match service {
            "sonarr" => "/series",
            "radarr" => "/movie",
            _ => "/artist",
        };
        let v = ctx
            .write(service, Method::POST, path, &[], Some(&body))
            .await?;
        Ok(if ctx.dry_run {
            json!({ "would_send": would(service, "POST", path, &[], Some(&body)) })
        } else {
            json!({ "added": v, "search_triggered": a.flag("search") })
        })
    })
}

// ---------- arr_update ----------

/// Fusion à plat des champs demandés dans l'objet courant ; un champ inconnu ou `id` est refusé (faute de frappe = pas d'écriture).
fn merge(current: &Value, changes: &Map<String, Value>) -> Result<Value> {
    let mut o = current
        .as_object()
        .cloned()
        .ok_or_else(|| bad("The current object is not a JSON object."))?;
    for (k, v) in changes {
        if k == "id" || !o.contains_key(k) {
            return Err(bad(format!(
                "Field `{k}` is unknown for this object or cannot be changed."
            )));
        }
        o.insert(k.clone(), v.clone());
    }
    Ok(Value::Object(o))
}

fn obj_path(resource: &str, id: i64) -> String {
    format!("/{resource}/{id}")
}

fn update(a: Args, ctx: Arc<Ctx>) -> BoxFut<Result<Value>> {
    boxed(async move {
        let (service, resource) = (a.text("service"), a.text("resource"));
        check_resource(service, resource, None)?;
        let path = obj_path(resource, positive(&a, "id")?);
        let changes = a.obj("changes");
        if changes.is_empty() {
            return Err(bad("`changes` must not be empty."));
        }
        no_mask(&Value::Object(changes.clone()), "changes")?;
        let previous = match ctx.client.get(service, &path, &[]).await {
            Ok(v) => Some(v),
            Err(e) if ctx.dry_run && e.is(ErrorType::NotFound) => None,
            Err(e) => return Err(e),
        };
        let Some(previous) = previous else {
            return Ok(
                json!({ "exists": false, "warning": "The object does not exist: a real call would fail and change nothing." }),
            );
        };
        let body = merge(&previous, &changes)?;
        let v = ctx
            .write(service, Method::PUT, &path, &[], Some(&body))
            .await?;
        Ok(if ctx.dry_run {
            json!({ "exists": true, "previous": previous, "would_send": would(service, "PUT", &path, &[], Some(&body)) })
        } else {
            json!({ "previous": previous, "updated": v })
        })
    })
}

// ---------- arr_command ----------

fn command_body(service: &str, name: &str, params: &Map<String, Value>) -> Result<Value> {
    let (_, _, spec) = COMMANDS
        .iter()
        .find(|(s, n, _)| *s == service && *n == name)
        .ok_or_else(|| {
            bad(format!(
                "Command `{name}` is not allowed on `{service}`. Allowed: {}.",
                allowed_commands(service)
            ))
        })?;
    if let Some(k) = params.keys().find(|k| !spec.iter().any(|(p, _, _)| p == k)) {
        return Err(bad(format!("Parameter `{k}` is not accepted by `{name}`.")));
    }
    let mut body = Map::new();
    body.insert("name".into(), json!(name));
    for (p, list, required) in spec.iter() {
        match params.get(*p) {
            None if *required => return Err(bad(format!("`{name}` requires `params.{p}`."))),
            None => {}
            Some(v) if *list => {
                body.insert((*p).into(), json!(ints_of(v, p)?));
            }
            Some(v) => {
                body.insert(
                    (*p).into(),
                    json!(v.as_i64().filter(|n| *n >= 0).ok_or_else(|| bad(format!(
                        "`params.{p}` must be a non-negative integer."
                    )))?),
                );
            }
        }
    }
    Ok(Value::Object(body))
}

fn allowed_commands(service: &str) -> String {
    COMMANDS
        .iter()
        .filter(|(s, _, _)| *s == service)
        .map(|(_, n, _)| *n)
        .collect::<Vec<_>>()
        .join(", ")
}

fn command(a: Args, ctx: Arc<Ctx>) -> BoxFut<Result<Value>> {
    boxed(async move {
        let service = a.text("service");
        let body = command_body(service, a.text("name"), &a.obj("params"))?;
        let v = ctx
            .write(service, Method::POST, "/command", &[], Some(&body))
            .await?;
        Ok(if ctx.dry_run {
            json!({ "would_send": would(service, "POST", "/command", &[], Some(&body)) })
        } else {
            json!({ "command": v })
        })
    })
}

// ---------- arr_grab_release ----------

/// Corps du grab. Quand Sonarr ne reconnaît pas la série d'un titre (« Unable to find matching series and
/// episodes, will need to be manually provided »), il accepte (source `ReleaseController`, vérifiée) :
/// - `seriesId` seul : il déduit les épisodes du titre avec cette série (pack de saison compris) ;
/// - `shouldOverride` avec `seriesId` + `episodeIds` + `quality` + `languages` (la ligne d'`arr_releases`) pour
///   choisir les épisodes, comme « Override and grab » de l'interface.
///
/// Sonarr seulement ; sans ces champs le corps reste `{guid, indexerId}`.
fn grab_body(service: &str, a: &Args) -> Result<Value> {
    let guid = a.text("guid");
    if guid.trim().is_empty() {
        return Err(bad("`guid` is required."));
    }
    let mut body = json!({ "guid": guid, "indexerId": positive(a, "indexerId")? });
    let given = |k: &str| a.0.get(k).is_some_and(|v| !v.is_null());
    if !["seriesId", "episodeIds", "quality", "languages"]
        .iter()
        .any(|k| given(k))
    {
        return Ok(body);
    }
    if service != "sonarr" {
        return Err(bad(
            "`seriesId`, `episodeIds`, `quality` and `languages` are only accepted for `sonarr`.",
        ));
    }
    if !given("seriesId") {
        return Err(bad("`seriesId` is required to map a release by hand."));
    }
    body["seriesId"] = json!(positive(a, "seriesId")?);
    if !["episodeIds", "quality", "languages"]
        .iter()
        .any(|k| given(k))
    {
        return Ok(body);
    }
    let missing: Vec<&str> = ["episodeIds", "quality", "languages"]
        .into_iter()
        .filter(|k| !given(k))
        .collect();
    if !missing.is_empty() {
        return Err(bad(format!(
            "Choosing episodes needs `episodeIds`, `quality` and `languages` together (copy the last two from the arr_releases row); missing: {}.",
            missing.join(", ")
        )));
    }
    let quality = a.obj("quality");
    let languages = a.list("languages");
    if quality.is_empty() || languages.is_empty() || !languages.iter().all(Value::is_object) {
        return Err(bad("`quality` must be the release's quality object and `languages` its list of language objects, as returned by arr_releases."));
    }
    body["shouldOverride"] = json!(true);
    body["episodeIds"] = json!(ints_of(&a.0["episodeIds"], "episodeIds")?);
    body["quality"] = Value::Object(quality);
    body["languages"] = Value::Array(languages);
    Ok(body)
}

fn grab_release(a: Args, ctx: Arc<Ctx>) -> BoxFut<Result<Value>> {
    boxed(async move {
        let service = a.text("service");
        check_service(service, &ARR4)?;
        let body = grab_body(service, &a)?;
        // Prowlarr télécharge par POST /search, les autres par POST /release.
        let path = if service == "prowlarr" {
            "/search"
        } else {
            "/release"
        };
        let v = ctx
            .write(service, Method::POST, path, &[], Some(&body))
            .await?;
        Ok(if ctx.dry_run {
            json!({ "would_send": would(service, "POST", path, &[], Some(&body)) })
        } else {
            json!({ "grabbed": v })
        })
    })
}

// ---------- arr_create / arr_test ----------

fn config_body(a: &Args) -> Result<Value> {
    let body = a.obj("body");
    if body.is_empty() {
        return Err(bad("`body` must not be empty."));
    }
    if body.get("id").is_some_and(|i| i != &json!(0)) {
        return Err(bad(
            "`body.id` must be absent or 0: use arr_update to change an existing object.",
        ));
    }
    let body = Value::Object(body);
    no_mask(&body, "body")?;
    Ok(body)
}

fn create(a: Args, ctx: Arc<Ctx>) -> BoxFut<Result<Value>> {
    boxed(async move {
        let (service, resource) = (a.text("service"), a.text("resource"));
        check_resource(service, resource, Some(&CREATABLE))?;
        let (body, path) = (config_body(&a)?, format!("/{resource}"));
        let v = ctx
            .write(service, Method::POST, &path, &[], Some(&body))
            .await?;
        Ok(if ctx.dry_run {
            json!({ "would_send": would(service, "POST", &path, &[], Some(&body)) })
        } else {
            json!({ "created": v })
        })
    })
}

fn test(a: Args, ctx: Arc<Ctx>) -> BoxFut<Result<Value>> {
    boxed(async move {
        let (service, resource) = (a.text("service"), a.text("resource"));
        check_resource(service, resource, Some(&TESTABLE))?;
        let (body, path) = (config_body(&a)?, format!("/{resource}/test"));
        let v = ctx
            .write(service, Method::POST, &path, &[], Some(&body))
            .await?;
        Ok(if ctx.dry_run {
            json!({ "would_send": would(service, "POST", &path, &[], Some(&body)) })
        } else {
            json!({ "ok": true, "result": v })
        })
    })
}

// ---------- arr_delete ----------

/// Noms des options de suppression propres à chaque ressource : (suppression de fichiers, exclusion d'import).
fn delete_options(resource: &str) -> Option<(&'static str, &'static str)> {
    match resource {
        "series" | "artist" | "album" => Some(("deleteFiles", "addImportListExclusion")),
        "movie" => Some(("deleteFiles", "addImportExclusion")),
        _ => None,
    }
}

fn delete_query(resource: &str, delete_files: bool, exclusion: bool) -> Result<Query> {
    match delete_options(resource) {
        Some((f, x)) => Ok(vec![(f.into(), delete_files.to_string()), (x.into(), exclusion.to_string())]),
        None if delete_files || exclusion => Err(bad(format!("`delete_files` and `add_import_exclusion` do not apply to `{resource}`; leave them false."))),
        None => Ok(vec![]),
    }
}

fn delete(a: Args, ctx: Arc<Ctx>) -> BoxFut<Result<Value>> {
    boxed(async move {
        let (service, resource) = (a.text("service"), a.text("resource"));
        check_resource(service, resource, None)?;
        let path = obj_path(resource, positive(&a, "id")?);
        let query = delete_query(
            resource,
            a.flag("delete_files"),
            a.flag("add_import_exclusion"),
        )?;
        // Instantané avant suppression : permet de recréer l'objet ; en dry-run un objet absent n'arrête pas l'aperçu.
        let snapshot = match ctx.client.get(service, &path, &[]).await {
            Ok(v) => Some(v),
            Err(e) if ctx.dry_run && e.is(ErrorType::NotFound) => None,
            Err(e) => return Err(e),
        };
        ctx.write(service, Method::DELETE, &path, &query, None)
            .await?;
        Ok(if ctx.dry_run {
            json!({ "exists": snapshot.is_some(), "would_delete": snapshot, "would_send": would(service, "DELETE", &path, &query, None) })
        } else {
            json!({ "deleted": snapshot })
        })
    })
}

// ---------- arr_queue_remove ----------

fn queue_remove(a: Args, ctx: Arc<Ctx>) -> BoxFut<Result<Value>> {
    boxed(async move {
        let service = a.text("service");
        check_service(service, &ARR3)?;
        let path = obj_path("queue", positive(&a, "id")?);
        let query: Query = vec![
            (
                "removeFromClient".into(),
                a.flag("remove_from_client").to_string(),
            ),
            ("blocklist".into(), a.flag("blocklist").to_string()),
        ];
        ctx.write(service, Method::DELETE, &path, &query, None)
            .await?;
        Ok(if ctx.dry_run {
            json!({ "would_send": would(service, "DELETE", &path, &query, None) })
        } else {
            json!({ "removed": true, "id": a.int("id") })
        })
    })
}

// ---------- arr_terminate_stream ----------

/// Tracearr n'accepte qu'un UUID de session (8-4-4-4-12 hexadécimaux) : le reste est refusé avant requête.
fn stream_id(a: &Args) -> Result<&str> {
    let id = a.text("id");
    let groups: Vec<&str> = id.split('-').collect();
    let uuid = groups.iter().map(|g| g.len()).eq([8, 4, 4, 4, 12])
        && groups
            .iter()
            .all(|g| g.chars().all(|c| c.is_ascii_hexdigit()));
    if !uuid {
        return Err(bad(
            "`id` must be a stream session id, a UUID such as 3f2b8c1e-0a4d-4e6f-9b7a-1c2d3e4f5a6b (the `id` of arr_list tracearr streams).",
        ));
    }
    Ok(id)
}

fn terminate_stream(a: Args, ctx: Arc<Ctx>) -> BoxFut<Result<Value>> {
    boxed(async move {
        let path = format!("/streams/{}/terminate", stream_id(&a)?);
        let v = ctx
            .write("tracearr", Method::POST, &path, &[], None)
            .await?;
        Ok(if ctx.dry_run {
            json!({ "would_send": would("tracearr", "POST", &path, &[], None) })
        } else {
            json!({ "terminated": true, "result": v })
        })
    })
}

// ---------- contrôles de portée (avant de déranger un humain) et confirmations ----------

fn pre_add(_: &Config, a: &Args) -> Result<()> {
    add_body(a.text("service"), a).map(|_| ())
}
/// Un objet lu est masqué : le recopier renverrait `[redacted]` au service à la place du vrai secret.
fn no_mask(v: &Value, arg: &str) -> Result<()> {
    if crate::redact::contains_mask(v) {
        return Err(bad(format!("`{arg}` contains the redaction mask \"{}\": nothing was sent. Do not copy an object you read back into a write: send only the fields you change, and never a secret field you did not set yourself.", crate::redact::MASK)));
    }
    Ok(())
}

fn pre_update(_: &Config, a: &Args) -> Result<()> {
    check_resource(a.text("service"), a.text("resource"), None)?;
    positive(a, "id")?;
    no_mask(&Value::Object(a.obj("changes")), "changes")
}
fn pre_command(_: &Config, a: &Args) -> Result<()> {
    command_body(a.text("service"), a.text("name"), &a.obj("params")).map(|_| ())
}
fn pre_grab(_: &Config, a: &Args) -> Result<()> {
    check_service(a.text("service"), &ARR4)?;
    grab_body(a.text("service"), a).map(|_| ())
}
fn pre_create(_: &Config, a: &Args) -> Result<()> {
    check_resource(a.text("service"), a.text("resource"), Some(&CREATABLE))?;
    config_body(a).map(|_| ())
}
fn pre_test(_: &Config, a: &Args) -> Result<()> {
    check_resource(a.text("service"), a.text("resource"), Some(&TESTABLE))?;
    config_body(a).map(|_| ())
}
fn pre_delete(_: &Config, a: &Args) -> Result<()> {
    check_resource(a.text("service"), a.text("resource"), None)?;
    positive(a, "id")?;
    delete_query(
        a.text("resource"),
        a.flag("delete_files"),
        a.flag("add_import_exclusion"),
    )
    .map(|_| ())
}
fn pre_queue(_: &Config, a: &Args) -> Result<()> {
    check_service(a.text("service"), &ARR3)?;
    positive(a, "id").map(|_| ())
}
/// Pas d'argument `service` : le contrôle générique ne voit pas Tracearr, on le fait ici, avant tout humain.
fn pre_terminate(cfg: &Config, a: &Args) -> Result<()> {
    stream_id(a)?;
    if cfg.service("tracearr").is_none() {
        return Err(ArrError::new(
            ErrorType::NotConfigured,
            "tracearr is not configured on this server: nothing was sent.",
        )
        .hint("Do not retry. The operator can set TRACEARR_URL and TRACEARR_API_KEY."));
    }
    crate::policy::check_service_writable(cfg, "tracearr")
}

fn conf_add(a: &Args) -> String {
    format!(
        "Add to {} (profile {}, folder {}, search after add: {})?",
        a.text("service"),
        a.int("qualityProfileId").unwrap_or(0),
        a.text("rootFolderPath"),
        a.flag("search")
    )
}
fn conf_update(a: &Args) -> String {
    format!(
        "Change {} #{} on {} with {}?",
        a.text("resource"),
        a.int("id").unwrap_or(0),
        a.text("service"),
        a.0.get("changes").map(Value::to_string).unwrap_or_default()
    )
}
fn conf_command(a: &Args) -> String {
    format!(
        "Run {} on {} with {}?",
        a.text("name"),
        a.text("service"),
        a.0.get("params")
            .map(Value::to_string)
            .unwrap_or_else(|| "no parameters".into())
    )
}
fn conf_grab(a: &Args) -> String {
    let mapping = match (a.int("seriesId"), a.0.get("episodeIds")) {
        (Some(s), Some(e)) => format!(", mapped by hand to series #{s} episodes {e}"),
        (Some(s), None) => format!(", mapped by hand to series #{s}"),
        _ => String::new(),
    };
    format!(
        "Download release {} from indexer #{} through {}{}?",
        a.text("guid"),
        a.int("indexerId").unwrap_or(0),
        a.text("service"),
        mapping
    )
}
fn conf_create(a: &Args) -> String {
    format!("Create a {} on {}?", a.text("resource"), a.text("service"))
}
fn conf_test(a: &Args) -> String {
    format!("Test a {} on {}?", a.text("resource"), a.text("service"))
}
fn conf_delete(a: &Args) -> String {
    format!(
        "DELETE {} #{} on {} (delete files: {}, import exclusion: {}). This cannot be undone.",
        a.text("resource"),
        a.int("id").unwrap_or(0),
        a.text("service"),
        a.flag("delete_files"),
        a.flag("add_import_exclusion")
    )
}
fn conf_queue(a: &Args) -> String {
    format!(
        "Remove queue item #{} from {} (remove from download client: {}, blocklist: {})?",
        a.int("id").unwrap_or(0),
        a.text("service"),
        a.flag("remove_from_client"),
        a.flag("blocklist")
    )
}
fn conf_terminate(a: &Args) -> String {
    format!(
        "Stop playback of stream {} on Tracearr? The user's session is cut.",
        a.text("id")
    )
}

// ---------- catalogue ----------

#[allow(clippy::too_many_arguments)]
fn tool(
    name: &'static str,
    title: &'static str,
    description: &str,
    input: Vec<Field>,
    access: Access,
    idempotent: bool,
    handler: Handler,
    precheck: Precheck,
    confirmation: Confirmation,
) -> ToolDef {
    ToolDef {
        name,
        title,
        description: description.into(),
        input: Schema::new(input),
        access,
        idempotent,
        open_world: true,
        handler,
        precheck: Some(precheck),
        confirmation: Some(confirmation),
    }
}

pub fn tools(_cfg: &Config) -> Vec<ToolDef> {
    let svc = |l: &[&str]| enumeration("service", l).desc("Target application.");
    let res = |d: &str| string("resource").desc(d);
    let id = |d: &str| integer("id").min(1).desc(d);
    let body = |d: &str| unknown_record("body").desc(d);
    let all_res = "series, movie, artist, album, indexer, downloadclient, tag, qualityprofile (those the service has).";
    vec![
        tool(
            "arr_add",
            "Add a series, movie or artist",
            "Add an item from an arr_lookup result (`item`). NO automatic search is started unless `search` is true; whatever the lookup object carries is overridden. Lidarr also needs `metadataProfileId`.",
            vec![
                svc(&ARR3),
                unknown_record("item").desc("A lookup result (must carry tvdbId, tmdbId or foreignArtistId)."),
                integer("qualityProfileId").min(1).desc("Quality profile id (arr_list qualityprofile)."),
                string("rootFolderPath").desc("Root folder path (arr_list rootfolder)."),
                integer("metadataProfileId").opt().min(1).desc("Lidarr only."),
                boolean("monitored").opt().desc("Monitor the item (default true)."),
                any("tags").opt().desc("List of tag ids."),
                boolean("search").opt().desc("Start a search right after adding (default false)."),
            ],
            Access::Write,
            false,
            add,
            pre_add,
            conf_add,
        ),
        tool(
            "arr_update",
            "Update an object",
            "Read the object, merge `changes` (top-level fields that already exist; typos are refused), PUT it back. Returns `previous` and `updated`.",
            vec![svc(&ARR4), res(all_res), id("Object id."), unknown_record("changes").desc("Fields to change, e.g. {\"monitored\":false,\"tags\":[1]}.")],
            Access::Write,
            true,
            update,
            pre_update,
            conf_update,
        ),
        tool(
            "arr_command",
            "Run a command",
            "POST /command with a whitelisted command name (e.g. RefreshSeries, SeriesSearch, EpisodeSearch, SeasonSearch, MissingEpisodeSearch, RefreshMovie, MoviesSearch, MissingMoviesSearch, RefreshArtist, ArtistSearch, AlbumSearch, MissingAlbumSearch, RssSync). Parameters are validated per command; an unknown name is refused before any request.",
            vec![svc(&ARR3), string("name").desc("Command name."), unknown_record("params").opt().desc("Command parameters, e.g. {\"seriesId\":12} or {\"movieIds\":[3]}.")],
            Access::Write,
            false,
            command,
            pre_command,
            conf_command,
        ),
        tool(
            "arr_grab_release",
            "Download a release",
            "Send a release found with arr_releases (or arr_search_indexers on Prowlarr) to the download client. Sonarr only, when it answers `Unable to find matching series and episodes` (e.g. `Unknown Series` on a season pack): pass `seriesId` alone (Sonarr reads the episodes from the title), or `seriesId` + `episodeIds` + `quality` + `languages` (the last two copied from the arr_releases row) to choose the episodes.",
            vec![
                svc(&ARR4),
                string("guid").desc("Release guid."),
                integer("indexerId").min(1).desc("Indexer id of the release."),
                integer("seriesId").min(1).opt().desc("Sonarr only: series the release belongs to."),
                array_of("episodeIds", Ty::Int { min: Some(1), max: None }).opt().desc("Sonarr only, with seriesId, quality and languages: episode ids the release covers (1 to 500)."),
                unknown_record("quality").opt().desc("Sonarr only, with episodeIds: the `quality` object of the arr_releases row."),
                array_of("languages", Ty::Unknown).opt().desc("Sonarr only, with episodeIds: the `languages` list of the arr_releases row."),
            ],
            Access::Write,
            false,
            grab_release,
            pre_grab,
            conf_grab,
        ),
        tool(
            "arr_create",
            "Create a tag, profile, indexer or download client",
            "Create a tag, qualityprofile (not Prowlarr), indexer or downloadclient. `body` is the full object (id absent or 0).",
            vec![svc(&ARR4), res("tag, qualityprofile, indexer or downloadclient."), body("Object to create.")],
            Access::Write,
            false,
            create,
            pre_create,
            conf_create,
        ),
        tool(
            "arr_test",
            "Test an indexer or download client",
            "POST /indexer/test or /downloadclient/test with a full configuration; nothing is saved.",
            vec![svc(&ARR4), res("indexer or downloadclient."), body("Configuration to test.")],
            Access::Write,
            true,
            test,
            pre_test,
            conf_test,
        ),
        tool(
            "arr_delete",
            "Delete an object",
            "Delete one object. `delete_files` (remove media files from disk) and `add_import_exclusion` default to false and only apply to series, movie, artist and album. Dry-run returns `exists`, the object and the request.",
            vec![svc(&ARR4), res(all_res), id("Object id."), boolean("delete_files").opt().desc("Also delete the files on disk (default false)."), boolean("add_import_exclusion").opt().desc("Add an import exclusion so it is not re-added (default false).")],
            Access::Destructive,
            true,
            delete,
            pre_delete,
            conf_delete,
        ),
        tool(
            "arr_queue_remove",
            "Remove a queue item",
            "Remove one item from the download queue. `remove_from_client` and `blocklist` default to false.",
            vec![svc(&ARR3), id("Queue item id."), boolean("remove_from_client").opt().desc("Also remove it from the download client (default false)."), boolean("blocklist").opt().desc("Blocklist the release (default false).")],
            Access::Destructive,
            true,
            queue_remove,
            pre_queue,
            conf_queue,
        ),
        tool(
            "arr_terminate_stream",
            "Terminate a Tracearr stream",
            "Tracearr only: cut a user's playback (POST /api/v1/public/streams/{id}/terminate).",
            vec![string("id").desc("Session UUID: the `id` of arr_list tracearr streams.")],
            Access::Destructive,
            false,
            terminate_stream,
            pre_terminate,
            conf_terminate,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: Value) -> Args {
        Args(v.as_object().unwrap().clone())
    }
    fn add_args(service: &str, extra: Value) -> Args {
        let mut m = json!({ "service": service, "item": { "id": 0, "tvdbId": 1, "tmdbId": 2, "foreignArtistId": "x", "addOptions": { "searchForMissingEpisodes": true, "searchForMovie": true, "searchForMissingAlbums": true } }, "qualityProfileId": 1, "rootFolderPath": "/r", "metadataProfileId": 1 });
        m.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        args(m)
    }

    #[test]
    fn add_never_searches_by_default() {
        for (s, flag) in [
            ("sonarr", "searchForMissingEpisodes"),
            ("radarr", "searchForMovie"),
            ("lidarr", "searchForMissingAlbums"),
        ] {
            let b = add_body(s, &add_args(s, json!({}))).unwrap();
            assert_eq!(b["addOptions"][flag], json!(false), "{s}");
            assert!(b.get("id").is_none());
            let b = add_body(s, &add_args(s, json!({ "search": true }))).unwrap();
            assert_eq!(b["addOptions"][flag], json!(true), "{s}");
        }
        let b = add_body("sonarr", &add_args("sonarr", json!({ "search": true }))).unwrap();
        assert_eq!(
            b["addOptions"]["searchForCutoffUnmetEpisodes"],
            json!(false)
        );
    }

    #[test]
    fn add_refuses_incomplete() {
        assert!(add_body(
            "sonarr",
            &args(json!({ "item": {}, "qualityProfileId": 1, "rootFolderPath": "/r" }))
        )
        .is_err());
        assert!(add_body("lidarr", &args(json!({ "item": { "foreignArtistId": "x" }, "qualityProfileId": 1, "rootFolderPath": "/r" }))).is_err());
        assert!(add_body("prowlarr", &add_args("prowlarr", json!({}))).is_err());
    }

    #[test]
    fn update_merges_and_rejects_unknown() {
        let cur = json!({ "id": 3, "monitored": true, "tags": [], "title": "t" });
        let ch = json!({ "monitored": false }).as_object().unwrap().clone();
        let m = merge(&cur, &ch).unwrap();
        assert_eq!(
            m,
            json!({ "id": 3, "monitored": false, "tags": [], "title": "t" })
        );
        assert!(merge(&cur, json!({ "monitord": false }).as_object().unwrap()).is_err());
        assert!(merge(&cur, json!({ "id": 9 }).as_object().unwrap()).is_err());
    }

    #[test]
    fn grab_maps_a_release_by_hand_for_sonarr_only() {
        let g = |v: Value| {
            grab_body(
                v["service"].as_str().unwrap(),
                &Args(v.as_object().unwrap().clone()),
            )
        };
        let base = json!({ "service": "sonarr", "guid": "g", "indexerId": 2 });
        let with = |extra: Value| {
            let mut m = base.clone();
            m.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            m
        };
        let full = json!({ "seriesId": 7, "episodeIds": [11, 12], "quality": { "quality": { "id": 7 } }, "languages": [{ "id": 1 }] });
        // Sans rattachement : corps inchangé.
        assert_eq!(
            g(base.clone()).unwrap(),
            json!({ "guid": "g", "indexerId": 2 })
        );
        // seriesId seul : Sonarr déduit les épisodes du titre.
        assert_eq!(
            g(with(json!({ "seriesId": 7 }))).unwrap(),
            json!({ "guid": "g", "indexerId": 2, "seriesId": 7 })
        );
        // Épisodes choisis : override complet.
        let body = g(with(full.clone())).unwrap();
        assert_eq!(body["shouldOverride"], true);
        assert_eq!(body["episodeIds"], json!([11, 12]));
        // Refus : épisodes sans qualité/langues, sans série, valeurs invalides, autre service.
        assert!(g(with(json!({ "seriesId": 7, "episodeIds": [1] }))).is_err());
        assert!(g(with(json!({ "episodeIds": [1] }))).is_err());
        assert!(g(with(json!({ "seriesId": 0 }))).is_err());
        assert!(g(with(json!({ "seriesId": 7, "episodeIds": [], "quality": { "q": 1 }, "languages": [{ "id": 1 }] }))).is_err());
        assert!(g(with(
            json!({ "seriesId": 7, "episodeIds": [1], "quality": { "q": 1 }, "languages": ["fr"] })
        ))
        .is_err());
        let mut radarr = with(full);
        radarr["service"] = json!("radarr");
        assert!(g(radarr).is_err());
    }

    #[test]
    fn command_whitelist() {
        let p = |v: Value| v.as_object().unwrap().clone();
        assert!(command_body("sonarr", "DeleteLogFiles", &p(json!({}))).is_err());
        assert!(command_body("radarr", "SeriesSearch", &p(json!({ "seriesId": 1 }))).is_err());
        assert!(command_body("sonarr", "SeriesSearch", &p(json!({}))).is_err());
        assert!(command_body(
            "sonarr",
            "SeriesSearch",
            &p(json!({ "seriesId": 1, "x": 1 }))
        )
        .is_err());
        assert!(command_body("sonarr", "SeriesSearch", &p(json!({ "seriesId": "1" }))).is_err());
        assert_eq!(
            command_body("sonarr", "SeriesSearch", &p(json!({ "seriesId": 4 }))).unwrap(),
            json!({ "name": "SeriesSearch", "seriesId": 4 })
        );
        assert_eq!(
            command_body("radarr", "MoviesSearch", &p(json!({ "movieIds": [1, 2] }))).unwrap(),
            json!({ "name": "MoviesSearch", "movieIds": [1, 2] })
        );
        assert!(command_body("radarr", "MoviesSearch", &p(json!({ "movieIds": [] }))).is_err());
        assert_eq!(
            command_body("lidarr", "MissingAlbumSearch", &p(json!({}))).unwrap(),
            json!({ "name": "MissingAlbumSearch" })
        );
    }

    #[test]
    fn resources_and_delete_query() {
        assert!(check_resource("sonarr", "series", None).is_ok());
        assert!(check_resource("radarr", "series", None).is_err());
        assert!(check_resource("prowlarr", "qualityprofile", None).is_err());
        assert!(check_resource("sonarr", "rootfolder", None).is_err());
        assert!(check_resource("sonarr", "series", Some(&CREATABLE)).is_err());
        let q = delete_query("movie", false, false).unwrap();
        assert_eq!(
            q,
            vec![
                ("deleteFiles".to_string(), "false".to_string()),
                ("addImportExclusion".to_string(), "false".to_string())
            ]
        );
        assert_eq!(
            delete_query("series", true, true).unwrap()[1].0,
            "addImportListExclusion"
        );
        assert!(delete_query("tag", true, false).is_err());
        assert!(delete_query("tag", false, false).unwrap().is_empty());
    }

    #[test]
    fn stream_id_validation() {
        assert!(stream_id(&args(
            json!({ "id": "3f2b8c1e-0a4d-4e6f-9b7a-1c2d3e4f5a6b" })
        ))
        .is_ok());
        // Tracearr (MESURÉ v3.1.7) : un id qui n'est pas un UUID -> 400 « Invalid session ID » ; refusé avant requête.
        assert!(stream_id(&args(json!({ "id": "a1-b_2" }))).is_err());
        assert!(stream_id(&args(
            json!({ "id": "3f2b8c1e-0a4d-4e6f-9b7a-1c2d3e4f5a6" })
        ))
        .is_err());
        assert!(stream_id(&args(
            json!({ "id": "3f2b8c1e00a4d-4e6f-9b7a-1c2d3e4f5a6b" })
        ))
        .is_err());
        assert!(stream_id(&args(json!({ "id": "../x" }))).is_err());
        assert!(stream_id(&args(json!({ "id": "" }))).is_err());
    }

    #[test]
    fn config_body_refuses_existing_id() {
        assert!(config_body(&args(json!({ "body": { "label": "a" } }))).is_ok());
        assert!(config_body(&args(json!({ "body": { "id": 5 } }))).is_err());
        assert!(config_body(&args(json!({ "body": {} }))).is_err());
    }
}
