//! Table des applications prises en charge : une ligne par service (préfixe d'API, authentification,
//! variables d'environnement, ressources lisibles en liste blanche).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Auth {
    /// En-tête `X-Api-Key: <clé>` (*arr).
    ApiKeyHeader(&'static str),
    /// `Authorization: Bearer <clé>` (Tracearr).
    Bearer,
}

#[derive(Debug)]
pub struct ServiceDef {
    pub name: &'static str,
    pub display: &'static str,
    pub api_prefix: &'static str,
    pub auth: Auth,
    pub url_env: &'static str,
    pub key_env: &'static str,
    /// (nom de ressource, chemin relatif au préfixe) : seules ressources acceptées par `arr_list`/`arr_get`.
    pub resources: &'static [(&'static str, &'static str)],
    /// Second préfixe (Tracearr : `/api/v1/public`) et motifs de chemins qui y vont (`*` = un segment).
    pub v1_prefix: Option<&'static str>,
    pub v1_paths: &'static [&'static str],
}

const ARR: Auth = Auth::ApiKeyHeader("X-Api-Key");

// Ressources communes aux *arr v3 (Sonarr, Radarr) et v1 (Lidarr).
const SONARR: &[(&str, &str)] = &[
    ("series", "series"),
    ("episode", "episode"),
    ("episodefile", "episodefile"),
    ("qualityprofile", "qualityprofile"),
    ("qualitydefinition", "qualitydefinition"),
    ("languageprofile", "languageprofile"),
    ("customformat", "customformat"),
    ("tag", "tag"),
    ("rootfolder", "rootfolder"),
    ("indexer", "indexer"),
    ("downloadclient", "downloadclient"),
    ("importlist", "importlist"),
    ("notification", "notification"),
    ("delayprofile", "delayprofile"),
    ("releaseprofile", "releaseprofile"),
    ("blocklist", "blocklist"),
    ("naming", "config/naming"),
    ("mediamanagement", "config/mediamanagement"),
    ("diskspace", "diskspace"),
    ("importlistexclusion", "importlistexclusion"),
    ("remotepathmapping", "remotepathmapping"),
    ("task", "system/task"),
    ("update", "update"),
];

const RADARR: &[(&str, &str)] = &[
    ("movie", "movie"),
    ("moviefile", "moviefile"),
    ("collection", "collection"),
    ("qualityprofile", "qualityprofile"),
    ("qualitydefinition", "qualitydefinition"),
    ("customformat", "customformat"),
    ("tag", "tag"),
    ("rootfolder", "rootfolder"),
    ("indexer", "indexer"),
    ("downloadclient", "downloadclient"),
    ("importlist", "importlist"),
    ("importlistexclusion", "exclusions"),
    ("notification", "notification"),
    ("delayprofile", "delayprofile"),
    ("blocklist", "blocklist"),
    ("naming", "config/naming"),
    ("mediamanagement", "config/mediamanagement"),
    ("diskspace", "diskspace"),
    ("remotepathmapping", "remotepathmapping"),
    ("task", "system/task"),
    ("update", "update"),
];

const LIDARR: &[(&str, &str)] = &[
    ("artist", "artist"),
    ("album", "album"),
    ("track", "track"),
    ("trackfile", "trackfile"),
    ("qualityprofile", "qualityprofile"),
    ("metadataprofile", "metadataprofile"),
    ("qualitydefinition", "qualitydefinition"),
    ("customformat", "customformat"),
    ("tag", "tag"),
    ("rootfolder", "rootfolder"),
    ("indexer", "indexer"),
    ("downloadclient", "downloadclient"),
    ("importlist", "importlist"),
    ("notification", "notification"),
    ("delayprofile", "delayprofile"),
    ("blocklist", "blocklist"),
    ("naming", "config/naming"),
    ("mediamanagement", "config/mediamanagement"),
    ("diskspace", "diskspace"),
    ("importlistexclusion", "importlistexclusion"),
    ("remotepathmapping", "remotepathmapping"),
    ("task", "system/task"),
    ("update", "update"),
];

const PROWLARR: &[(&str, &str)] = &[
    ("indexer", "indexer"),
    ("indexerstats", "indexerstats"),
    ("indexerproxy", "indexerproxy"),
    ("application", "applications"),
    ("appprofile", "appprofile"),
    ("downloadclient", "downloadclient"),
    ("notification", "notification"),
    ("tag", "tag"),
    ("indexerstatus", "indexerstatus"),
    ("task", "system/task"),
    ("update", "update"),
];

const TRACEARR: &[(&str, &str)] = &[
    ("streams", "streams"),
    ("history", "history"),
    ("violations", "violations"),
    ("servers", "servers"),
    ("users", "users"),
    ("libraries", "libraries"),
    ("recently-added", "recently-added"),
    ("watched-media", "watched-media"),
    ("top-media", "top-media"),
    ("stats", "stats"),
];

pub static SERVICES: [ServiceDef; 5] = [
    ServiceDef {
        name: "sonarr",
        display: "Sonarr (TV)",
        api_prefix: "/api/v3",
        auth: ARR,
        url_env: "SONARR_URL",
        key_env: "SONARR_API_KEY",
        resources: SONARR,
        v1_prefix: None,
        v1_paths: &[],
    },
    ServiceDef {
        name: "radarr",
        display: "Radarr (movies)",
        api_prefix: "/api/v3",
        auth: ARR,
        url_env: "RADARR_URL",
        key_env: "RADARR_API_KEY",
        resources: RADARR,
        v1_prefix: None,
        v1_paths: &[],
    },
    ServiceDef {
        name: "lidarr",
        display: "Lidarr (music)",
        api_prefix: "/api/v1",
        auth: ARR,
        url_env: "LIDARR_URL",
        key_env: "LIDARR_API_KEY",
        resources: LIDARR,
        v1_prefix: None,
        v1_paths: &[],
    },
    ServiceDef {
        name: "prowlarr",
        display: "Prowlarr (indexers)",
        api_prefix: "/api/v1",
        auth: ARR,
        url_env: "PROWLARR_URL",
        key_env: "PROWLARR_API_KEY",
        resources: PROWLARR,
        v1_prefix: None,
        v1_paths: &[],
    },
    ServiceDef {
        name: "tracearr",
        display: "Tracearr (playback monitoring)",
        api_prefix: "/api/v2/public",
        auth: Auth::Bearer,
        url_env: "TRACEARR_URL",
        key_env: "TRACEARR_API_KEY",
        resources: TRACEARR,
        v1_prefix: Some("/api/v1/public"),
        v1_paths: &["stats", "streams/*/terminate", "activity", "health"],
    },
];

pub fn get(name: &str) -> Option<&'static ServiceDef> {
    SERVICES.iter().find(|s| s.name == name)
}

pub fn names() -> Vec<&'static str> {
    SERVICES.iter().map(|s| s.name).collect()
}

/// `motif` couvre-t-il `path` ? Comparaison par segments, `*` = un segment quelconque, le motif est un préfixe.
pub(crate) fn pattern_covers(pattern: &str, path: &str) -> bool {
    let (p, s): (Vec<&str>, Vec<&str>) = (pattern.split('/').collect(), path.split('/').collect());
    p.len() <= s.len() && p.iter().zip(&s).all(|(a, b)| *a == "*" || a == b)
}

impl ServiceDef {
    /// Chemin relatif d'une ressource de la liste blanche.
    pub fn resource(&self, name: &str) -> Option<&'static str> {
        self.resources
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, p)| *p)
    }

    pub fn resource_names(&self) -> Vec<&'static str> {
        self.resources.iter().map(|(n, _)| *n).collect()
    }

    /// Préfixe d'API à utiliser pour ce chemin relatif (déjà normalisé).
    pub fn prefix_for(&self, rel: &str) -> &'static str {
        match self.v1_prefix {
            Some(v1) if self.v1_paths.iter().any(|p| pattern_covers(p, rel)) => v1,
            _ => self.api_prefix,
        }
    }

    pub fn is_arr(&self) -> bool {
        matches!(self.auth, Auth::ApiKeyHeader(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_complete() {
        assert_eq!(
            names(),
            vec!["sonarr", "radarr", "lidarr", "prowlarr", "tracearr"]
        );
        assert_eq!(get("lidarr").unwrap().api_prefix, "/api/v1");
        assert_eq!(
            get("sonarr").unwrap().resource("naming"),
            Some("config/naming")
        );
        assert!(get("radarr").unwrap().resource("series").is_none());
        assert_eq!(get("lidarr").unwrap().resource("task"), Some("system/task"));
        assert_eq!(
            get("prowlarr").unwrap().resource("indexerstatus"),
            Some("indexerstatus")
        );
        // Tracearr n'a pas de liste `media` (404 mesuré sur 3.1.7) : retirée de la liste blanche.
        assert!(get("tracearr").unwrap().resource("media").is_none());
        assert!(get("readarr").is_none());
    }

    #[test]
    fn tracearr_v1_routing() {
        let t = get("tracearr").unwrap();
        assert_eq!(t.prefix_for("stats"), "/api/v1/public");
        assert_eq!(t.prefix_for("stats/plays"), "/api/v1/public");
        assert_eq!(t.prefix_for("streams/abc/terminate"), "/api/v1/public");
        assert_eq!(t.prefix_for("streams"), "/api/v2/public");
        assert_eq!(t.prefix_for("statsx"), "/api/v2/public");
        assert_eq!(t.prefix_for("activity"), "/api/v1/public");
        assert_eq!(t.prefix_for("health"), "/api/v1/public");
        assert_eq!(get("sonarr").unwrap().prefix_for("stats"), "/api/v3");
    }
}
