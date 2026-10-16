use super::*;
use std::collections::BTreeMap;

struct WidgetIndex {
    ids: BTreeMap<String, usize>,
    keys: BTreeMap<String, usize>,
}

impl WidgetIndex {
    fn new(widgets: &[Value]) -> Self {
        let mut index = Self {
            ids: BTreeMap::new(),
            keys: BTreeMap::new(),
        };
        for (position, widget) in widgets.iter().enumerate() {
            index.observe(position, widget);
        }
        index
    }

    fn observe(&mut self, position: usize, widget: &Value) {
        for (field, entries) in [("id", &mut self.ids), ("managedKey", &mut self.keys)] {
            if let Some(key) = widget[field].as_str() {
                entries
                    .entry(key.to_owned())
                    .and_modify(|first| *first = (*first).min(position))
                    .or_insert(position);
            }
        }
    }

    fn locate(&self, bundled: &Value) -> Option<usize> {
        let id = bundled["id"].as_str().and_then(|id| self.ids.get(id));
        let key = bundled["managedKey"]
            .as_str()
            .and_then(|key| self.keys.get(key));
        id.into_iter().chain(key).copied().min()
    }
}

enum WidgetEdit {
    Keep,
    Replace(usize, Value),
    Append(Value),
}

impl WidgetEdit {
    fn plan(widgets: &[Value], bundled: &Value, position: Option<usize>) -> Self {
        if let Some(position) = position {
            let (Some(current), Some(template)) =
                (widgets[position].as_object(), bundled.as_object())
            else {
                return Self::Keep;
            };
            let mut projected = current.clone();
            projected.insert("managed".into(), Value::Bool(true));
            if let Some(key) = bundled["managedKey"].as_str() {
                projected.insert("managedKey".into(), Value::String(key.into()));
            }
            for field in ["componentType", "definitionId"] {
                if let Some(value) = template.get(field) {
                    projected.insert(field.into(), value.clone());
                }
            }
            let mut options = template
                .get("options")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            if let Some(personal) = current.get("options").and_then(Value::as_object) {
                options.extend(personal.clone());
            }
            if bundled["managedKey"].as_str() == Some("trips.tire-pressure-timeline") {
                for field in ["page", "chartId", "showPicker"] {
                    if let Some(value) = template
                        .get("options")
                        .and_then(|options| options.get(field))
                    {
                        options.insert(field.into(), value.clone());
                    }
                }
            }
            projected.insert("options".into(), Value::Object(options));
            if &projected == current {
                Self::Keep
            } else {
                Self::Replace(position, Value::Object(projected))
            }
        } else {
            let bottom = widgets
                .iter()
                .filter_map(|widget| {
                    widget["layout"]["y"]
                        .as_i64()
                        .zip(widget["layout"]["h"].as_i64())
                })
                .map(|(y, h)| y + h)
                .max()
                .unwrap_or(0);
            let mut appended = bundled.clone();
            if let Some(layout) = appended
                .as_object_mut()
                .and_then(|object| object.get_mut("layout"))
                .and_then(Value::as_object_mut)
            {
                layout.insert("y".into(), Value::from(bottom));
            }
            Self::Append(appended)
        }
    }
}

pub(super) fn merge(
    mut config: Value,
    bundled: &[&Value],
    retired: &[(&str, &str)],
) -> Option<Value> {
    let widgets = config.get_mut("widgets")?.as_array_mut()?;
    let count = widgets.len();
    widgets.retain(|widget| {
        !retired.iter().any(|(key, id)| {
            widget["managedKey"].as_str() == Some(*key) || widget["id"].as_str() == Some(*id)
        })
    });
    let mut changed = count != widgets.len();
    if changed
        && retired
            .iter()
            .any(|(key, _)| *key == "trips.tire-pressure-timeline")
    {
        compact_canonical_trips_layout(widgets);
    }
    let mut index = WidgetIndex::new(widgets);
    for template in bundled {
        match WidgetEdit::plan(widgets, template, index.locate(template)) {
            WidgetEdit::Keep => {}
            WidgetEdit::Replace(position, value) => {
                widgets[position] = value;
                index = WidgetIndex::new(widgets);
                changed = true;
            }
            WidgetEdit::Append(value) => {
                index.observe(widgets.len(), &value);
                widgets.push(value);
                changed = true;
            }
        }
    }
    changed.then_some(config)
}

struct Seed<'a> {
    id: Uuid,
    slug: String,
    name: String,
    config: Value,
    source: &'a str,
}

impl<'a> Seed<'a> {
    fn decode(id: &str, source: &'a str) -> anyhow::Result<Self> {
        let id = id.parse()?;
        let config: Value = serde_json::from_str(source)?;
        let name = config["name"].as_str().unwrap_or("Dashboard").to_owned();
        let slug = config["slug"].as_str().unwrap_or("dashboard").to_owned();
        let config =
            dashboard_config_with_metadata(config, id, None, &slug, &name, None, true, true);
        Ok(Self {
            id,
            slug,
            name,
            config,
            source,
        })
    }

    async fn apply_system(&self, pool: &sqlx::PgPool) -> anyhow::Result<()> {
        sqlx::query(UPSERT_SYSTEM_DEFAULT_SQL)
            .bind(self.id)
            .bind(&self.slug)
            .bind(&self.name)
            .bind(&self.config)
            .bind(BUNDLED_DASHBOARD_BASELINE_REVISION)
            .execute(pool)
            .await?;
        Ok(())
    }

    async fn apply_personal(&self, pool: &sqlx::PgPool) -> anyhow::Result<()> {
        let original: Value = serde_json::from_str(self.source)?;
        let Some(slug) = original["slug"].as_str() else {
            return Ok(());
        };
        let managed = original["widgets"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|widget| widget["managed"].as_bool() == Some(true))
            .collect::<Vec<_>>();
        let retired = retired_managed_widgets(slug);
        if managed.is_empty() && retired.is_empty() {
            return Ok(());
        }
        let rows = sqlx::query_as::<_, (Uuid, Value)>(
            "SELECT id, config FROM panda.dashboards WHERE owner_id IS NOT NULL AND slug = $1",
        )
        .bind(slug)
        .fetch_all(pool)
        .await?;
        for (id, config) in rows {
            if let Some(next) = merge_managed_widgets(config, &managed, retired) {
                sqlx::query("UPDATE panda.dashboards SET config = $1, updated_at = NOW() WHERE id = $2 AND owner_id IS NOT NULL").bind(next).bind(id).execute(pool).await?;
            }
        }
        Ok(())
    }
}

pub(super) async fn seed(pool: &sqlx::PgPool, defaults: &[(&str, &str)]) -> anyhow::Result<()> {
    let mut applied = Vec::with_capacity(defaults.len());
    for (id, source) in defaults {
        let seed = Seed::decode(id, source)?;
        seed.apply_system(pool).await?;
        applied.push(seed);
    }
    for seed in applied {
        seed.apply_personal(pool).await?;
    }
    Ok(())
}
