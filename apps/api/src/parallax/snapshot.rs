//! Pure snapshot planning; database effects are applied only after decoding.
use super::*;

pub(super) struct NetworkSnapshot {
    pub overall_state: Option<i32>,
    pub active_transport: Option<i32>,
    pub wifi: WifiState,
    pub connected: bool,
    pub cellular: CellularState,
}

pub(super) struct EfficiencySnapshot {
    pub reference_wh_per_km: Option<i32>,
    pub learned_wh_per_km: Option<i32>,
    pub ranges: BTreeMap<String, i32>,
}

pub(super) struct CurvePoint {
    pub index: i32,
    pub at: DateTime<Utc>,
    pub session: Option<Uuid>,
    pub power_kw: Option<f64>,
    pub soc: Option<f64>,
}

pub(super) enum Snapshot {
    Network(NetworkSnapshot),
    Efficiency(EfficiencySnapshot),
    Mass(Option<i32>),
    Parked(Vec<(&'static str, ParkedEnergyWindow)>),
    Breakdown {
        total_kwh: Option<f64>,
        power_kw: Option<f64>,
        state: Option<i32>,
    },
    Pack(f64),
    Curve {
        points: Vec<CurvePoint>,
        latest_power: Option<(DateTime<Utc>, f64)>,
    },
    Remaining(i32),
    Status(String),
    Cold {
        available: Option<i32>,
        limited: Option<i32>,
        impact_km: Option<f64>,
    },
    Canonical,
    Empty,
    Ignored,
    Rejected,
}

impl Snapshot {
    pub fn decode(
        topic: &str,
        payload: &[u8],
        context: &crate::ingestion::worker::ActiveSessionContext,
    ) -> Result<Self> {
        macro_rules! charging_schema {
            ($message:ty) => {
                match <$message>::decode(payload) {
                    Ok(value) => value,
                    Err(_) => return Ok(Self::Rejected),
                }
            };
        }
        Ok(match topic {
            "vehicle.network.state" => {
                let value = NetworkState::decode(payload)?;
                let mut wifi = value.wifi.unwrap_or_default();
                let mut cellular = value.cellular.unwrap_or_default();
                wifi.rssi_dbm = without_signal_sentinel(wifi.rssi_dbm);
                cellular.signal_dbm = without_signal_sentinel(cellular.signal_dbm);
                cellular.access_technology =
                    cellular.access_technology.filter(|value| value.len() <= 16);
                Self::Network(NetworkSnapshot {
                    overall_state: value.overall_state,
                    active_transport: value.active_transport,
                    connected: wifi.status == Some(2) && wifi.rssi_dbm.is_some(),
                    wifi,
                    cellular,
                })
            }
            "dynamics.vehicle.efficiency" => {
                let value = EfficiencyState::decode(payload)?;
                let ranges = value
                    .mode_ranges
                    .into_iter()
                    .filter_map(|range| {
                        Some((range.mode?.to_string(), range.full_charge_range_km?))
                    })
                    .collect();
                Self::Efficiency(EfficiencySnapshot {
                    reference_wh_per_km: value.reference_wh_per_km,
                    learned_wh_per_km: value.learned_wh_per_km,
                    ranges,
                })
            }
            "dynamics.vehicle.mass_estimate" => Self::Mass(
                MassEstimate::decode(payload)?
                    .estimated_mass_kg
                    .filter(|mass| (1000..=10_000).contains(mass)),
            ),
            "energy_edge_compute.graphs.parked_energy_distributions" => {
                let value = ParkedEnergyDistributions::decode(payload)?;
                Self::Parked(
                    [
                        ("24h", value.hours_24),
                        ("8h", value.hours_8),
                        ("since_parked", value.since_parked),
                    ]
                    .into_iter()
                    .filter_map(|(window, sample)| Some((window, sample?)))
                    .collect(),
                )
            }
            "energy_edge_compute.graphs.charge_session_breakdown" => {
                let value = ChargeBreakdown::decode(payload)?;
                let total_kwh = f64_opt(value.total_kwh).filter(|value| *value >= 0.0);
                // A present invalid primary power does not select the fallback.
                let power_kw = value
                    .current_power_kw
                    .map(f64::from)
                    .or_else(|| value.fallback_power_kw.map(f64::from))
                    .filter(|value| value.is_finite() && (0.0..=500.0).contains(value));
                if total_kwh.is_none() && power_kw.is_none() && value.charging_state.is_none() {
                    Self::Empty
                } else {
                    Self::Breakdown {
                        total_kwh,
                        power_kw,
                        state: value.charging_state,
                    }
                }
            }
            "energy.high_voltage.battery_state" => charging_schema!(HvBatteryState)
                .charge_state
                .and_then(|state| state.pack_energy_kwh)
                .filter(|value| value.is_finite() && (0.0..=500.0).contains(value))
                .map_or(Self::Empty, Self::Pack),
            "energy_edge_compute.graphs.charging_graph_global" => {
                let value = charging_schema!(ChargingGraphGlobal);
                let points: Vec<_> = value
                    .segments
                    .into_iter()
                    .enumerate()
                    .filter_map(|(index, segment)| {
                        let at = Utc.timestamp_millis_opt(segment.start_unix_ms?).single()?;
                        let power_kw = segment
                            .power_kw
                            .filter(|value| value.is_finite() && (0.0..=500.0).contains(value))
                            .map(f64::from);
                        let soc = segment
                            .soc
                            .filter(|value| (0..=100).contains(value))
                            .map(f64::from);
                        if power_kw.is_none() && soc.is_none() {
                            return None;
                        }
                        Some(CurvePoint {
                            index: index as i32,
                            at,
                            session: matching_active_session(context, at),
                            power_kw,
                            soc,
                        })
                    })
                    .collect();
                if points.is_empty() {
                    Self::Empty
                } else {
                    let latest_power = points
                        .iter()
                        .filter_map(|point| Some((point.at, point.power_kw?)))
                        .fold(None, |latest, candidate| match latest {
                            Some((at, _)) if at > candidate.0 => latest,
                            _ => Some(candidate),
                        });
                    Self::Curve {
                        points,
                        latest_power,
                    }
                }
            }
            "charging.session.time_estimation" => charging_schema!(ChargingTimeEstimation)
                .remaining_seconds
                .filter(|seconds| (0..=172_800).contains(seconds))
                .map_or(Self::Empty, |seconds| Self::Remaining((seconds + 59) / 60)),
            "charging.session.status" => {
                let value = charging_schema!(ChargingStatus);
                let fields = [
                    value.plug_connection_status,
                    value.display_status,
                    value.evse_type,
                ];
                if fields.iter().all(Option::is_none) {
                    Self::Empty
                } else {
                    let [plug, display, evse] = fields.map(|field| {
                        field.map_or_else(|| "unknown".into(), |value| value.to_string())
                    });
                    Self::Status(format!("plug={plug};display={display};evse={evse}"))
                }
            }
            "energy_edge_compute.graphs.cold_weather_soc" => {
                let value = ColdWeatherSoc::decode(payload)?;
                Self::Cold {
                    available: value.available_soc_pct,
                    limited: value.cold_limited_soc_pct,
                    impact_km: f64_opt(value.cold_range_impact_km),
                }
            }
            topic if is_canonical_telemetry_topic(topic) => Self::Canonical,
            _ => Self::Ignored,
        })
    }
}
