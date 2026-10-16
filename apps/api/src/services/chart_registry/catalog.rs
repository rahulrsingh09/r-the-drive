use std::collections::BTreeMap;

use super::*;

#[derive(Deserialize)]
#[serde(from = "Value")]
struct SeedInput {
    slug: String,
    name: String,
    description: Option<String>,
    enabled: bool,
    config: Value,
}

impl From<Value> for SeedInput {
    fn from(value: Value) -> Self {
        Self {
            slug: value
                .get("slug")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            name: value
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("Chart")
                .to_owned(),
            description: value
                .get("description")
                .and_then(Value::as_str)
                .map(str::to_owned),
            enabled: value
                .get("enabled")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            config: value.get("definition").cloned().unwrap_or(Value::Null),
        }
    }
}

impl SeedInput {
    async fn apply(self, pool: &PgPool) -> Result<(), sqlx::Error> {
        sqlx::query(UPSERT_BUNDLED_CHART_SQL)
            .bind(self.slug)
            .bind(self.name)
            .bind(self.description)
            .bind(self.enabled)
            .bind(self.config)
            .bind(BUNDLED_CHART_BASELINE_REVISION)
            .execute(pool)
            .await?;
        Ok(())
    }
}

pub(super) async fn reconcile(pool: &PgPool) -> anyhow::Result<()> {
    for slug in RETIRED_BUNDLED_CHART_SLUGS {
        sqlx::query(DELETE_RETIRED_BUNDLED_CHART_SQL)
            .bind(slug)
            .execute(pool)
            .await?;
    }
    // Retirement and each successful row remain committed if a later statement fails.
    let inputs: Vec<SeedInput> = serde_json::from_str(BUNDLED_CHART_DEFAULTS)?;
    for input in inputs {
        input.apply(pool).await?;
    }
    Ok(())
}

#[derive(Default)]
struct SlugRows {
    system: Option<ChartRecord>,
    owned: Option<ChartRecord>,
    last_personal: Option<ChartRecord>,
}

impl SlugRows {
    fn include(&mut self, row: ChartRecord, user: Uuid) {
        if row.owner_id.is_none() {
            self.system.get_or_insert(row);
        } else {
            if row.owner_id == Some(user) && self.owned.is_none() {
                self.owned = Some(row.clone());
            }
            self.last_personal = Some(row);
        }
    }

    fn resolve(self, user: Uuid, admin: bool) -> ChartManagerEntry {
        let personal_override = self.owned.or(self.last_personal);
        let effective = personal_override
            .as_ref()
            .or(self.system.as_ref())
            .expect("a slug group contains at least one row")
            .clone();
        let personal = effective.owner_id == Some(user);
        let overrides = personal && self.system.is_some();
        let permissions = ChartPermissions {
            read: true,
            edit: admin || personal && !effective.is_locked,
            duplicate: true,
            reset: overrides,
            restore: admin,
            delete: personal,
            lock: admin,
        };
        ChartManagerEntry {
            effective,
            system_base: self.system,
            personal_override,
            origin: if overrides {
                "override"
            } else if personal {
                "personal"
            } else {
                "system"
            }
            .into(),
            permissions,
        }
    }
}

pub(super) fn merge(rows: Vec<ChartRecord>, user: Uuid, admin: bool) -> Vec<ChartManagerEntry> {
    let mut groups: BTreeMap<String, SlugRows> = BTreeMap::new();
    for row in rows {
        groups
            .entry(row.slug.clone())
            .or_default()
            .include(row, user);
    }
    groups
        .into_values()
        .map(|group| group.resolve(user, admin))
        .collect()
}
