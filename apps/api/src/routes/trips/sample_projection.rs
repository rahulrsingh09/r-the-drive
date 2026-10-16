use super::*;

#[derive(Serialize)]
pub(super) struct Powered<T> {
    #[serde(flatten)]
    values: T,
    estimated_net_power_kw: Option<f64>,
    power_source: TripPowerSource,
}

pub(super) fn powered<T>(
    rows: Vec<T>,
    timestamp: impl Fn(&T) -> DateTime<Utc>,
    evidence: &TripPowerData,
) -> Vec<Powered<T>> {
    rows.into_iter()
        .map(|values| Powered {
            estimated_net_power_kw: evidence.estimated_at(timestamp(&values)),
            power_source: evidence.source(),
            values,
        })
        .collect()
}

pub(super) struct DetailEvidence {
    samples: TripDetailSamples,
    power: TripPowerMetadata,
}

impl DetailEvidence {
    pub fn cached(payload: TripDetailCachePayload) -> Self {
        Self {
            samples: payload.samples,
            power: payload.power,
        }
    }

    pub fn sampled(rows: Vec<DetailSampleRow>, power: TripPowerData) -> Self {
        let samples = TripDetailSamples {
            elapsed_s: rows.iter().map(|r| r.elapsed_s).collect(),
            lat: rows.iter().map(|r| r.lat).collect(),
            lng: rows.iter().map(|r| r.lng).collect(),
            altitude_m: rows.iter().map(|r| r.altitude_m).collect(),
            speed_mph: rows.iter().map(|r| r.speed_mph).collect(),
            power_kw: rows.iter().map(|r| r.power_kw).collect(),
            regen_power_kw: rows.iter().map(|r| r.regen_power_kw).collect(),
            estimated_net_power_kw: rows
                .iter()
                .map(|r| power.estimated_at(r.bucket_ts))
                .collect(),
            battery_level: rows.iter().map(|r| r.battery_level).collect(),
            outside_temp_c: rows.iter().map(|r| r.outside_temp_c).collect(),
            cabin_temp_c: rows.iter().map(|r| r.cabin_temp_c).collect(),
            driver_temp_c: rows.iter().map(|r| r.driver_temp_c).collect(),
            hvac_active: rows.iter().map(|r| r.hvac_active).collect(),
            tire_fl_psi: rows.iter().map(|r| r.tire_fl_psi).collect(),
            tire_fr_psi: rows.iter().map(|r| r.tire_fr_psi).collect(),
            tire_rl_psi: rows.iter().map(|r| r.tire_rl_psi).collect(),
            tire_rr_psi: rows.iter().map(|r| r.tire_rr_psi).collect(),
        };
        Self {
            samples,
            power: power.metadata,
        }
    }

    pub fn project(
        mut self,
        trip: &TripRow,
        weather: &[TripWeatherRow],
    ) -> (TripDetailCachePayload, OutsideTemperatureResponse) {
        let raw = raw_outside_elapsed(&self.samples);
        merge_weather_samples(&mut self.samples, weather);
        let temperature = build_outside_temperature(trip, &self.samples, weather, &raw);
        (
            TripDetailCachePayload {
                samples: self.samples,
                power: self.power,
            },
            temperature,
        )
    }
}
