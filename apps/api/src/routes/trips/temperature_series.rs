use super::{
    OutsideTemperatureAttribution, OutsideTemperatureResponse, OutsideTemperatureSample,
    TripDetailSamples, TripRow, TripWeatherRow,
};
use chrono::{DateTime, Utc};
use std::collections::HashSet;

enum ObservedSources {
    Vehicle,
    Weather,
    Mixed,
    Neither,
}

impl ObservedSources {
    fn infer(raw_elapsed: &HashSet<i32>, weather: &[TripWeatherRow]) -> Self {
        match (!raw_elapsed.is_empty(), !weather.is_empty()) {
            (true, true) => Self::Mixed,
            (true, false) => Self::Vehicle,
            (false, true) => Self::Weather,
            (false, false) => Self::Neither,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Vehicle => "vehicle",
            Self::Weather => "open_meteo",
            Self::Mixed => "mixed",
            Self::Neither => "unavailable",
        }
    }
}

struct TemperatureEvidence<'a> {
    elapsed_s: i32,
    temperature_c: f64,
    raw: bool,
    weather: Option<&'a TripWeatherRow>,
}

impl TemperatureEvidence<'_> {
    fn project(self, started_at: DateTime<Utc>) -> OutsideTemperatureSample {
        let source = match (self.raw, self.weather.is_some()) {
            (false, true) => "open_meteo",
            _ => "vehicle",
        };
        // Raw provenance still permits the matched weather timestamp.
        let ts = match self.weather {
            Some(weather) => weather.sampled_at,
            None => started_at + chrono::Duration::seconds(i64::from(self.elapsed_s)),
        };
        OutsideTemperatureSample {
            elapsed_s: self.elapsed_s,
            ts,
            temperature_c: self.temperature_c,
            source,
        }
    }
}

pub(super) fn build(
    trip: &TripRow,
    samples: &TripDetailSamples,
    weather_rows: &[TripWeatherRow],
    raw_elapsed: &HashSet<i32>,
) -> OutsideTemperatureResponse {
    let source = trip.outside_temp_source.clone().unwrap_or_else(|| {
        ObservedSources::infer(raw_elapsed, weather_rows)
            .label()
            .into()
    });
    let evidence = samples
        .elapsed_s
        .iter()
        .zip(&samples.outside_temp_c)
        .filter_map(|(&elapsed_s, &temperature)| {
            let temperature_c = temperature?;
            let weather = weather_rows
                .iter()
                .min_by_key(|row| (row.elapsed_seconds - elapsed_s).abs())
                .filter(|row| (row.elapsed_seconds - elapsed_s).abs() <= 30);
            Some(TemperatureEvidence {
                elapsed_s,
                temperature_c,
                raw: raw_elapsed.contains(&elapsed_s),
                weather,
            })
        });
    let timeline_samples = evidence
        .map(|sample| sample.project(trip.started_at))
        .collect();
    OutsideTemperatureResponse {
        attribution: matches!(source.as_str(), "open_meteo" | "mixed").then_some(
            OutsideTemperatureAttribution {
                name: "Open-Meteo",
                url: "https://open-meteo.com/",
            },
        ),
        source,
        samples: timeline_samples,
    }
}
