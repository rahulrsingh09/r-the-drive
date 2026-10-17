use super::*;
use futures::TryStreamExt;

pub(super) struct CacheProjection {
    inventory: CacheInventory,
    status: &'static str,
    error: Option<&'static str>,
}

impl CacheProjection {
    pub async fn read(
        pool: &sqlx::PgPool,
        config: &crate::config::Config,
        vehicle: Uuid,
        failed: bool,
    ) -> Self {
        let inventory = sqlx::query_as::<_, VehicleImageRow>(
            "SELECT placement, design, size, resolution, url, overlays, metadata
             FROM panda.vehicle_images WHERE vehicle_id = $1",
        )
        .bind(vehicle)
        .fetch(pool)
        .try_fold(
            CacheInventory { total: 0, ready: 0 },
            |mut inventory, row| async move {
                let observed = CacheInventory::inspect(config, std::slice::from_ref(&row));
                inventory.total += observed.total;
                inventory.ready += observed.ready;
                Ok(inventory)
            },
        )
        .await
        // A failed read invalidates the entire observation, including any earlier rows.
        .unwrap_or(CacheInventory { total: 0, ready: 0 });
        Self {
            status: inventory.status(failed),
            inventory,
            error: failed.then_some("Rivian artwork repair failed; retry is scheduled."),
        }
    }

    pub async fn persist(self, pool: &sqlx::PgPool, vehicle: Uuid) {
        let _ = sqlx::query(
            r#"INSERT INTO panda.vehicle_artwork_cache_state
                  (vehicle_id, status, asset_count, ready_asset_count, last_repair_success_at, last_error, updated_at)
               VALUES ($1, $2, $3, $4, CASE WHEN $2 = 'ready' THEN now() ELSE NULL END, $5, now())
               ON CONFLICT (vehicle_id) DO UPDATE SET
                 status = EXCLUDED.status, asset_count = EXCLUDED.asset_count,
                 ready_asset_count = EXCLUDED.ready_asset_count,
                 last_repair_success_at = CASE WHEN EXCLUDED.status = 'ready' THEN now() ELSE panda.vehicle_artwork_cache_state.last_repair_success_at END,
                 last_error = EXCLUDED.last_error, updated_at = now()"#,
        )
        .bind(vehicle)
        .bind(self.status)
        .bind(self.inventory.total)
        .bind(self.inventory.ready)
        .bind(self.error)
        .execute(pool)
        .await;
    }
}

#[derive(Default)]
struct PlacementCandidates {
    first: Option<usize>,
    designs: BTreeMap<&'static str, usize>,
}

pub(super) struct ArtworkSelection<'a> {
    rows: &'a [VehicleImageRow],
    placements: BTreeMap<&'static str, PlacementCandidates>,
}

impl<'a> ArtworkSelection<'a> {
    pub fn new(rows: &'a [VehicleImageRow]) -> Self {
        let mut placements: BTreeMap<_, PlacementCandidates> = BTreeMap::new();
        for (index, row) in rows.iter().enumerate() {
            let candidates = placements
                .entry(normalize_image_placement(&row.placement))
                .or_default();
            candidates.first.get_or_insert(index);
            candidates
                .designs
                .entry(normalize_image_design(row.design.as_deref()))
                .or_insert(index);
        }
        Self { rows, placements }
    }

    fn preferred(
        &self,
        config: &crate::config::Config,
        vehicle_id: Uuid,
        placement: &str,
        design: &str,
    ) -> Option<String> {
        let candidates = self.placements.get(placement)?;
        let index = candidates
            .designs
            .get(design)
            .copied()
            .or(candidates.first)?;
        let row = &self.rows[index];
        // Resolve after selection: an unmirrored first candidate intentionally stays absent.
        resolved_image_url(config, vehicle_id, &row.url, &row.metadata)
    }

    pub fn render(
        &self,
        config: &crate::config::Config,
        vehicle_id: Uuid,
        cache: serde_json::Value,
    ) -> serde_json::Value {
        let all: Vec<_> = self.rows.iter().filter_map(|row| {
            let url = resolved_image_url(config, vehicle_id, &row.url, &row.metadata)?;
            Some(serde_json::json!({
                "placement":row.placement,"design":row.design,"size":row.size,"resolution":row.resolution,
                "url":url,"overlays":resolved_overlays(config,vehicle_id,&row.overlays,&row.metadata),"metadata":row.metadata,
            }))
        }).collect();
        let mut output = serde_json::Map::new();
        output.insert("all".into(), serde_json::json!(all));
        output.insert("cache".into(), cache);
        for placement in ["side", "overhead", "front", "rear"] {
            output.insert(
                placement.into(),
                serde_json::json!({
                    "dark":self.preferred(config,vehicle_id,placement,"dark"),
                    "light":self.preferred(config,vehicle_id,placement,"light"),
                }),
            );
        }
        serde_json::Value::Object(output)
    }
}

pub(super) struct CacheInventory {
    pub total: i32,
    pub ready: i32,
}

impl CacheInventory {
    pub fn inspect(config: &crate::config::Config, rows: &[VehicleImageRow]) -> Self {
        rows.iter()
            .filter(|row| row.url.starts_with("http://") || row.url.starts_with("https://"))
            .fold(Self { total: 0, ready: 0 }, |mut counts, row| {
                counts.total += 1;
                counts.ready += i32::from(metadata_has_local_mirror(config, &row.metadata));
                if let Some(overlays) = row
                    .metadata
                    .get("overlay_mirrors")
                    .and_then(serde_json::Value::as_array)
                {
                    for overlay in overlays {
                        counts.total += 1;
                        counts.ready += i32::from(mirrored_asset_is_valid(config, overlay));
                    }
                }
                counts
            })
    }

    pub fn status(&self, failed: bool) -> &'static str {
        match (self.total > 0 && self.ready == self.total, failed) {
            (true, _) => "ready",
            (false, true) => "failed",
            (false, false) => "pending",
        }
    }
}

pub(super) fn overlay_projection(
    config: &crate::config::Config,
    vehicle_id: Uuid,
    row_overlays: &serde_json::Value,
    metadata: &serde_json::Value,
) -> serde_json::Value {
    let Some(mirrors) = metadata
        .get("overlay_mirrors")
        .and_then(serde_json::Value::as_array)
    else {
        return serde_json::json!([]);
    };
    let mut first_by_source = BTreeMap::new();
    for mirror in mirrors {
        if let Some(source) = mirror.get("source_url").and_then(serde_json::Value::as_str) {
            first_by_source.entry(source).or_insert(mirror);
        }
    }
    let projected: Vec<_> = overlay_entries_from_value(row_overlays)
        .into_iter()
        .filter_map(|mut overlay| {
            let mirror = *first_by_source.get(overlay.url.as_str())?;
            let key = mirror
                .get("mirror_key")
                .and_then(serde_json::Value::as_str)?;
            if !metadata_has_local_mirror(config, mirror) {
                return None;
            }
            overlay.url = format!("/v1/vehicle-image-cache/{vehicle_id}/{key}");
            Some(overlay)
        })
        .collect();
    serde_json::to_value(projected).unwrap_or_else(|_| serde_json::json!([]))
}
