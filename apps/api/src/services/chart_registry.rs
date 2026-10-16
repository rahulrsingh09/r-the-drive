use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

#[path = "chart_registry/catalog.rs"]
mod catalog;
#[cfg(test)]
#[path = "chart_registry/contract_tests.rs"]
mod contract_tests;

pub const BUNDLED_CHART_BASELINE_REVISION: i32 = 5;

const BUNDLED_CHART_DEFAULTS: &str = include_str!("../../charts/defaults.json");
const RETIRED_BUNDLED_CHART_SLUGS: &[&str] = &["charge-session-curve"];
const DELETE_RETIRED_BUNDLED_CHART_SQL: &str =
    "DELETE FROM panda.charts WHERE owner_id IS NULL AND slug = $1";
const UPSERT_BUNDLED_CHART_SQL: &str =
    "INSERT INTO panda.charts (owner_id,slug,name,description,is_default,is_locked,is_enabled,config,baseline_revision) VALUES (NULL,$1,$2,$3,TRUE,FALSE,$4,$5,$6) ON CONFLICT (slug) WHERE owner_id IS NULL DO UPDATE SET name=EXCLUDED.name,description=EXCLUDED.description,is_enabled=EXCLUDED.is_enabled,config=EXCLUDED.config,baseline_revision=EXCLUDED.baseline_revision,updated_at=NOW() WHERE COALESCE(panda.charts.baseline_revision,0) < COALESCE(EXCLUDED.baseline_revision,0)";

/// Seed installation-wide chart rows without touching personal overrides.
/// The JSON file is generated from the canonical dashboard package defaults.
pub async fn seed_defaults(pool: &PgPool) -> anyhow::Result<()> {
    catalog::reconcile(pool).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn bundled_chart_baseline_advances_existing_system_rows_only() {
        assert_eq!(BUNDLED_CHART_BASELINE_REVISION, 5);
        assert!(UPSERT_BUNDLED_CHART_SQL.contains("VALUES (NULL"));
        assert!(UPSERT_BUNDLED_CHART_SQL.contains("ON CONFLICT (slug) WHERE owner_id IS NULL"));
        assert!(UPSERT_BUNDLED_CHART_SQL.contains(
            "COALESCE(panda.charts.baseline_revision,0) < COALESCE(EXCLUDED.baseline_revision,0)"
        ));
        assert!(DELETE_RETIRED_BUNDLED_CHART_SQL.contains("owner_id IS NULL"));
        assert!(DELETE_RETIRED_BUNDLED_CHART_SQL.contains("slug = $1"));
    }

    #[test]
    fn embedded_baseline_contains_the_complete_renderer_compatible_inventory() {
        let defaults: Vec<Value> = serde_json::from_str(BUNDLED_CHART_DEFAULTS).unwrap();
        assert_eq!(defaults.len(), 14);
        let slugs = defaults
            .iter()
            .map(|chart| chart["slug"].as_str().unwrap())
            .collect::<HashSet<_>>();
        assert_eq!(slugs.len(), defaults.len());
        assert!(!slugs.contains("charge-session-curve"));
        assert_eq!(RETIRED_BUNDLED_CHART_SLUGS, ["charge-session-curve"]);
        assert!(defaults.iter().all(|chart| {
            chart["definition"]["schemaVersion"] == 1
                && chart["definition"]["series"]
                    .as_array()
                    .is_some_and(|series| !series.is_empty())
        }));

        let battery = defaults
            .iter()
            .find(|chart| chart["slug"] == "battery-capacity-mileage")
            .unwrap();
        let fields = battery["definition"]["series"]
            .as_array()
            .unwrap()
            .iter()
            .map(|series| series["y"]["field"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(fields, ["usable_kwh", "odometer_miles"]);
        assert_eq!(battery["definition"]["series"][0]["fill"], true);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ChartRecord {
    pub id: Uuid,
    pub owner_id: Option<Uuid>,

    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub is_default: bool,
    pub is_locked: bool,
    pub is_enabled: bool,
    pub config: Value,
    pub baseline_revision: Option<i32>,

    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartPermissions {
    pub read: bool,
    pub edit: bool,

    pub duplicate: bool,
    pub reset: bool,
    pub restore: bool,
    pub delete: bool,
    pub lock: bool,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartManagerEntry {
    pub effective: ChartRecord,
    pub system_base: Option<ChartRecord>,
    pub personal_override: Option<ChartRecord>,
    pub origin: String,
    pub permissions: ChartPermissions,
}

pub fn merge_entries(rows: Vec<ChartRecord>, user_id: Uuid, admin: bool) -> Vec<ChartManagerEntry> {
    catalog::merge(rows, user_id, admin)
}
