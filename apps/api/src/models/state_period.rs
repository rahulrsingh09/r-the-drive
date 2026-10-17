use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// One contiguous period in which the vehicle was in a given coarse state.
///
/// The ingestion worker maintains exactly one open row (`ended_at IS NULL`)
/// per vehicle at any time.  When a state transition is detected the open row
/// is closed and a new one is opened.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct VehicleStatePeriod {
    pub id: i64,
    pub vehicle_id: Uuid,
    pub state: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub duration_seconds: Option<i32>,
}

/// Valid state values (mirrors the CHECK constraint in migration 0009).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VehicleState {
    Drive,
    Charging,
    Ready,
    Sleep,
    Offline,
    Updating,
    Unknown,
}

impl VehicleState {
    const VOCABULARY: &'static [(&'static str, Self)] = &[
        ("drive", Self::Drive),
        ("charging", Self::Charging),
        ("ready", Self::Ready),
        ("sleep", Self::Sleep),
        ("offline", Self::Offline),
        ("updating", Self::Updating),
        ("unknown", Self::Unknown),
    ];
}

impl std::fmt::Display for VehicleState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (text, _) = Self::VOCABULARY
            .iter()
            .find(|(_, state)| state == self)
            .expect("every vehicle state has a wire name");
        f.write_str(text)
    }
}

impl std::str::FromStr for VehicleState {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::VOCABULARY
            .iter()
            .find(|(text, _)| *text == value)
            .map(|(_, state)| state.clone())
            .ok_or_else(|| format!("unknown vehicle state {value:?}"))
    }
}

/// Firmware version history record.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct SoftwareVersion {
    pub id: i64,
    pub vehicle_id: Uuid,
    pub version: String,
    pub installed_at: DateTime<Utc>,
    pub observed_until: Option<DateTime<Utc>>,
}

/// Manual service / maintenance log entry.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ServiceEvent {
    pub id: Uuid,
    pub vehicle_id: Uuid,
    pub event_type: String,
    pub performed_at: DateTime<Utc>,
    pub odometer_mi: Option<f64>,
    pub cost_usd: Option<f64>,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
}
