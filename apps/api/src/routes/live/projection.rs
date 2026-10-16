use super::ActiveLiveSession;
use chrono::{DateTime, Utc};
use serde_json::{json, Map, Value};

enum LiveNumber {
    Decimal(f64),
    Minutes(i32),
}

impl LiveNumber {
    fn wire_value(self) -> Value {
        match self {
            Self::Decimal(value) => json!(value),
            Self::Minutes(value) => json!(value),
        }
    }
}

pub(super) struct FieldObservation {
    field: &'static str,
    value: Option<LiveNumber>,
    observed_at: Option<DateTime<Utc>>,
}

pub(super) struct SelectedField {
    field: &'static str,
    value: LiveNumber,
    observed_at: DateTime<Utc>,
}

impl FieldObservation {
    pub(super) fn select(self, now: DateTime<Utc>) -> Option<SelectedField> {
        let observed_at = self
            .observed_at
            .filter(|observed| *observed >= now - chrono::Duration::seconds(120))?;
        Some(SelectedField {
            field: self.field,
            value: self.value?,
            observed_at,
        })
    }
}

pub(super) fn observations(active: ActiveLiveSession) -> [FieldObservation; 5] {
    [
        FieldObservation {
            field: "power_kw",
            value: active.parallax_live_power_kw.map(LiveNumber::Decimal),
            observed_at: active.parallax_power_observed_at,
        },
        FieldObservation {
            field: "energy_kwh",
            value: active.parallax_total_charged_kwh.map(LiveNumber::Decimal),
            observed_at: active.parallax_total_energy_observed_at,
        },
        FieldObservation {
            field: "pack_energy_kwh",
            value: active.parallax_pack_energy_kwh.map(LiveNumber::Decimal),
            observed_at: active.parallax_pack_energy_observed_at,
        },
        FieldObservation {
            field: "thermal_energy_kwh",
            value: active.parallax_thermal_energy_kwh.map(LiveNumber::Decimal),
            observed_at: active.parallax_thermal_energy_observed_at,
        },
        FieldObservation {
            field: "time_remaining_min",
            value: active
                .parallax_time_remaining_minutes
                .map(LiveNumber::Minutes),
            observed_at: active.parallax_time_observed_at,
        },
    ]
}

pub(super) struct LiveSnapshot {
    object: Map<String, Value>,
    provenance: Map<String, Value>,
}

impl LiveSnapshot {
    pub(super) fn from_legacy(raw: Option<&str>) -> Self {
        let object = match raw.and_then(|raw| serde_json::from_str(raw).ok()) {
            Some(Value::Object(object)) => object,
            _ => Map::new(),
        };
        let legacy_at = object.get("ts").and_then(Value::as_str).map(str::to_owned);
        let provenance = ["power_kw", "energy_kwh", "time_remaining_min"]
            .into_iter()
            .filter(|field| object.get(*field).is_some_and(|value| !value.is_null()))
            .map(|field| {
                (
                    field.to_owned(),
                    json!({
                        "source":"legacy_charging_session",
                        "observed_at":legacy_at
                    }),
                )
            })
            .collect();
        Self { object, provenance }
    }

    pub(super) fn overlay(&mut self, selected: SelectedField) {
        self.object
            .insert(selected.field.into(), selected.value.wire_value());
        self.provenance.insert(
            selected.field.into(),
            json!({
                "source":"parallax",
                "observed_at":selected.observed_at
            }),
        );
    }

    pub(super) fn finish(mut self) -> Option<String> {
        if !self.provenance.is_empty() {
            self.object
                .insert("provenance".into(), self.provenance.into());
        }
        if self.object.is_empty() {
            None
        } else {
            serde_json::to_string(&self.object).ok()
        }
    }
}
