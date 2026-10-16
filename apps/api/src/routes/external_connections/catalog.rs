use super::{
    active_endpoint, connections, resolve_basemap_provider, ConnectionDefinition,
    ConnectionSettingsRow,
};

enum Source {
    Backup,
    OpenFreeMap,
    Configured,
}

pub(super) struct Presentation {
    pub enabled: bool,
    pub mode: String,
    pub endpoint: Option<String>,
    pub privacy_url: Option<&'static str>,
    pub terms_url: Option<&'static str>,
    pub attribution: Option<String>,
    pub attribution_url: Option<String>,
}

impl Presentation {
    pub(super) fn resolve(
        settings: &ConnectionSettingsRow,
        backup: &(bool, Option<String>),
        definition: &ConnectionDefinition,
    ) -> Self {
        let source = match settings.id.as_str() {
            connections::S3_BACKUP => Source::Backup,
            connections::BASEMAP if resolve_basemap_provider(settings) == "openfreemap" => {
                Source::OpenFreeMap
            }
            _ => Source::Configured,
        };
        let (enabled, mode, endpoint) = match source {
            Source::Backup => (backup.0, "custom".into(), backup.1.clone()),
            Source::OpenFreeMap | Source::Configured => (
                settings.is_active(),
                settings.mode.clone(),
                active_endpoint(settings),
            ),
        };
        let (privacy_url, terms_url, attribution, attribution_url) = match source {
            Source::OpenFreeMap => (
                Some("https://openfreemap.org/privacy/"),
                Some("https://openfreemap.org/tos/"),
                Some("OpenFreeMap © OpenMapTiles Data from OpenStreetMap".into()),
                Some("https://openfreemap.org/".into()),
            ),
            Source::Backup | Source::Configured => (
                definition.privacy_url,
                definition.terms_url,
                settings.attribution.clone(),
                settings.attribution_url.clone(),
            ),
        };
        Self {
            enabled,
            mode,
            endpoint,
            privacy_url,
            terms_url,
            attribution,
            attribution_url,
        }
    }
}
