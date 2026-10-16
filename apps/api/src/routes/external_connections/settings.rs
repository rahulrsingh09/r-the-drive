use super::{connections, normalize, UpdateConnectionBody};

pub(super) struct UpdatePlan {
    pub mode: String,
    pub forecast_url: Option<String>,
    pub archive_url: Option<String>,
    pub base_url: Option<String>,
    pub light_url_template: Option<String>,
    pub dark_url_template: Option<String>,
    pub attribution: Option<String>,
    pub attribution_url: Option<String>,
    pub request_identifier: Option<String>,
}

impl UpdatePlan {
    pub(super) fn resolve(id: &str, body: &UpdateConnectionBody) -> Self {
        let mode = match (body.enabled, body.mode.as_str()) {
            (false, _) => "disabled",
            (true, "hosted") => "remote",
            (true, mode) => mode,
        };
        let submitted = [
            &body.forecast_url,
            &body.archive_url,
            &body.base_url,
            &body.light_url_template,
            &body.dark_url_template,
            &body.attribution,
            &body.attribution_url,
        ];
        let defaults = if mode == "remote" {
            remote_defaults(id)
        } else {
            [None; 7]
        };
        let [forecast_url, archive_url, base_url, light_url_template, dark_url_template, attribution, attribution_url] =
            std::array::from_fn(|index| {
                defaults[index]
                    .map(str::to_owned)
                    .or_else(|| normalize(submitted[index].clone()))
            });
        Self {
            mode: mode.into(),
            forecast_url,
            archive_url,
            base_url,
            light_url_template,
            dark_url_template,
            attribution,
            attribution_url,
            request_identifier: normalize(body.request_identifier.clone()),
        }
    }
}

fn remote_defaults(id: &str) -> [Option<&'static str>; 7] {
    let (forecast, archive, base, light, dark, attribution, attribution_url) = match id {
        connections::OPEN_METEO => (
            Some("https://api.open-meteo.com/v1/forecast"),
            Some("https://archive-api.open-meteo.com/v1/archive"),
            None,
            None,
            None,
            Some("Weather data by Open-Meteo"),
            Some("https://open-meteo.com/"),
        ),
        connections::NOMINATIM => (
            None,
            None,
            Some("https://nominatim.openstreetmap.org"),
            None,
            None,
            Some("OpenStreetMap contributors"),
            Some("https://www.openstreetmap.org/copyright"),
        ),
        connections::BASEMAP => (
            None,
            None,
            None,
            Some("https://basemaps.cartocdn.com/light_all/{z}/{x}/{y}.png"),
            Some("https://basemaps.cartocdn.com/dark_all/{z}/{x}/{y}.png"),
            Some("OpenStreetMap contributors and CARTO"),
            Some("https://carto.com/attributions"),
        ),
        connections::ICONIFY => (
            None,
            None,
            Some("https://api.iconify.design"),
            None,
            None,
            Some("Iconify"),
            Some("https://iconify.design/"),
        ),
        _ => (None, None, None, None, None, None, None),
    };
    [
        forecast,
        archive,
        base,
        light,
        dark,
        attribution,
        attribution_url,
    ]
}
