use super::{connections, AppError, ConnectionSettingsRow, UpdateConnectionBody};

#[derive(Clone, Copy, PartialEq)]
enum Preference {
    Auto,
    Carto,
    OpenFreeMap,
}

impl Preference {
    fn parse(value: &str) -> Result<Self, AppError> {
        match value {
            "auto" => Ok(Self::Auto),
            "carto" => Ok(Self::Carto),
            "openfreemap" => Ok(Self::OpenFreeMap),
            _ => Err(AppError::Validation(
                "basemap_provider must be auto, openfreemap, or carto".into(),
            )),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Carto => "carto",
            Self::OpenFreeMap => "openfreemap",
        }
    }

    fn provider(self, key_available: bool) -> &'static str {
        match (self, key_available) {
            (Self::Carto, _) | (Self::Auto, true) => "carto",
            (Self::OpenFreeMap, _) | (Self::Auto, false) => "openfreemap",
        }
    }
}

fn submitted_or_retained_key(
    body: &UpdateConnectionBody,
    existing: Option<&ConnectionSettingsRow>,
) -> bool {
    let submitted = body
        .api_key
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty());
    let retained = !body.clear_api_key.unwrap_or(false)
        && existing.is_some_and(|settings| settings.api_key_encrypted.is_some());
    submitted || retained
}

fn select_update(
    body: &UpdateConnectionBody,
    existing: Option<&ConnectionSettingsRow>,
) -> Result<Preference, AppError> {
    let source = body
        .basemap_provider
        .as_deref()
        .or_else(|| existing.map(|settings| settings.basemap_provider.as_str()))
        .unwrap_or("auto");
    let requested = Preference::parse(source)?;
    let preference = match (requested, body.clear_api_key.unwrap_or(false)) {
        (Preference::Carto, true) => Preference::Auto,
        (preference, _) => preference,
    };
    if preference == Preference::Carto
        && body.enabled
        && matches!(body.mode.as_str(), "remote" | "hosted")
        && !submitted_or_retained_key(body, existing)
    {
        return Err(AppError::Validation(
            "a CARTO basemap key is required when CARTO is pinned".into(),
        ));
    }
    Ok(preference)
}

pub(super) fn update(
    id: &str,
    body: &UpdateConnectionBody,
    existing: Option<&ConnectionSettingsRow>,
) -> Result<Option<String>, AppError> {
    if id == connections::BASEMAP {
        return select_update(body, existing).map(|preference| Some(preference.name().into()));
    }
    match body.basemap_provider {
        None => Ok(None),
        Some(_) => Err(AppError::Validation(
            "basemap_provider is only supported for the map basemap connection".into(),
        )),
    }
}

pub(super) fn effective(
    id: &str,
    body: &UpdateConnectionBody,
    existing: &ConnectionSettingsRow,
) -> Result<String, AppError> {
    match (body.enabled, body.mode.as_str()) {
        (false, _) | (_, "disabled") => Ok("disabled".into()),
        (_, "custom") => Ok("custom".into()),
        _ => {
            let preference = update(id, body, Some(existing))?
                .expect("basemap provider is always present for the basemap connection");
            Ok(Preference::parse(&preference)?
                .provider(submitted_or_retained_key(body, Some(existing)))
                .into())
        }
    }
}

pub(super) fn stored(settings: &ConnectionSettingsRow) -> &'static str {
    match (settings.enabled, settings.mode.as_str()) {
        (false, _) | (_, "disabled") => "disabled",
        (_, "custom") => "custom",
        _ => Preference::parse(&settings.basemap_provider)
            .unwrap_or(Preference::Auto)
            .provider(settings.api_key_encrypted.is_some()),
    }
}
