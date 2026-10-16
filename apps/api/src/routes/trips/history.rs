use super::*;

const TRIP_COLUMNS: &str = r#"SELECT t.id, t.started_at, t.ended_at, t.duration_seconds, t.distance_miles, t.efficiency_wh_per_mile, t.max_speed_mph, t.drive_mode, t.soc_start, t.soc_end, t.start_lat, t.start_lng, t.end_lat, t.end_lng, COALESCE(sg.name, NULLIF(CONCAT_WS(', ', sa.road, sa.city), '')) AS start_place, COALESCE(eg.name, NULLIF(CONCAT_WS(', ', ea.road, ea.city), '')) AS end_place, sa.display_name AS start_address, ea.display_name AS end_address, t.outside_temp_c AS outside_temp_c, t.outside_temp_source AS outside_temp_source, COALESCE((SELECT jsonb_agg(jsonb_build_object('id', tt.id, 'vehicle_id', tt.vehicle_id, 'name', tt.name, 'color_token', tt.color_token, 'created_by', tt.created_by, 'created_at', tt.created_at, 'updated_at', tt.updated_at) ORDER BY lower(tt.name), tt.id) FROM panda.trip_tag_assignments tta JOIN panda.trip_tags tt ON tt.id=tta.tag_id WHERE tta.trip_id=t.id), '[]'::jsonb) AS tags"#;
const TAG_COLUMN: &str = r#"COALESCE((SELECT jsonb_agg(jsonb_build_object('id', tt.id, 'vehicle_id', tt.vehicle_id, 'name', tt.name, 'color_token', tt.color_token, 'created_by', tt.created_by, 'created_at', tt.created_at, 'updated_at', tt.updated_at) ORDER BY lower(tt.name), tt.id) FROM panda.trip_tag_assignments tta JOIN panda.trip_tags tt ON tt.id=tta.tag_id WHERE tta.trip_id=t.id), '[]'::jsonb) AS tags"#;

fn relations(user: usize) -> String {
    format!(
        r#"FROM panda.trips t LEFT JOIN panda.trip_user_annotations tua ON tua.trip_id = t.id AND tua.user_id = ${user} LEFT JOIN panda.geofences sg ON sg.id = COALESCE(tua.start_geofence_id, t.start_geofence_id) LEFT JOIN panda.geofences eg ON eg.id = COALESCE(tua.end_geofence_id, t.end_geofence_id) LEFT JOIN panda.addresses sa ON sa.id = COALESCE(tua.start_address_id, t.start_address_id) LEFT JOIN panda.addresses ea ON ea.id = COALESCE(tua.end_address_id, t.end_address_id)"#
    )
}

pub(super) async fn trip(
    pool: &sqlx::PgPool,
    vehicle: Uuid,
    id: Uuid,
    user: Uuid,
) -> Result<TripRow, AppError> {
    let sql = format!(
        "{TRIP_COLUMNS} {} WHERE t.id=$1 AND t.vehicle_id=$2",
        relations(3)
    );
    sqlx::query_as(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(id)
        .bind(vehicle)
        .bind(user)
        .fetch_optional(pool)
        .await?
        .ok_or(AppError::NotFound)
}

pub(super) struct HistoryPlan {
    vehicle: Uuid,
    user: Uuid,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    search: Option<String>,
    tags: TripTagFilter,
}

pub(super) struct Page {
    limit: i64,
    offset: i64,
}
impl Page {
    pub fn new(p: &TripListParams) -> Self {
        let limit = p.per_page.or(p.limit).unwrap_or(50).clamp(1, 200);
        let page = p.page.unwrap_or(1).max(1);
        Self {
            limit,
            offset: p.offset.unwrap_or((page - 1) * limit).max(0),
        }
    }
    fn response(self, rows: Vec<TripRow>, total: i64) -> serde_json::Value {
        serde_json::json!({"data":rows,"items":rows,"total":total,"limit":self.limit,
            "offset":self.offset,"page":self.offset/self.limit+1,"per_page":self.limit})
    }
}

impl HistoryPlan {
    pub fn new(vehicle: Uuid, user: Uuid, p: &TripListParams, tags: TripTagFilter) -> Self {
        let (from, to) = resolve_time_bounds(p.from, p.to, p.lifetime.unwrap_or(false), 90);
        let search = p
            .search
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(|v| v.replace('%', "\\%").replace('_', "\\_"));
        Self {
            vehicle,
            user,
            from,
            to,
            search,
            tags,
        }
    }

    fn selection(&self) -> String {
        format!(
            "SELECT t.id {} WHERE t.vehicle_id=$1 AND t.started_at>=$2 AND t.started_at<=$3 {} {}",
            relations(5),
            trip_search_predicate(4),
            sql_predicate("t", 6, 7, 8)
        )
    }

    pub async fn page(
        self,
        pool: &sqlx::PgPool,
        page: Page,
    ) -> Result<serde_json::Value, AppError> {
        let selection = self.selection();
        let rows_sql=format!("WITH selected AS ({selection}) {TRIP_COLUMNS} {} WHERE t.id IN (SELECT id FROM selected) ORDER BY t.started_at DESC LIMIT $9 OFFSET $10",relations(5));
        let total_sql = format!("WITH selected AS ({selection}) SELECT COUNT(*) FROM selected");
        let mut tx = pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
            .execute(&mut *tx)
            .await?;
        let rows = sqlx::query_as::<_, TripRow>(sqlx::AssertSqlSafe(rows_sql.as_str()))
            .bind(self.vehicle)
            .bind(self.from)
            .bind(self.to)
            .bind(self.search.as_deref())
            .bind(self.user)
            .bind(&self.tags.tag_ids)
            .bind(self.tags.match_all)
            .bind(self.tags.untagged)
            .bind(page.limit)
            .bind(page.offset)
            .fetch_all(&mut *tx)
            .await?;
        let total = sqlx::query_scalar(sqlx::AssertSqlSafe(total_sql.as_str()))
            .bind(self.vehicle)
            .bind(self.from)
            .bind(self.to)
            .bind(self.search.as_deref())
            .bind(self.user)
            .bind(&self.tags.tag_ids)
            .bind(self.tags.match_all)
            .bind(self.tags.untagged)
            .fetch_one(&mut *tx)
            .await?;
        tx.rollback().await?;
        Ok(page.response(rows, total))
    }

    pub async fn map(self, pool: &sqlx::PgPool) -> Result<TripMapResponse, AppError> {
        let sql=format!("WITH selected AS ({}) SELECT t.id,t.vehicle_id,t.started_at,t.ended_at,t.route_preview,{TAG_COLUMN} FROM panda.trips t WHERE t.id IN (SELECT id FROM selected) ORDER BY t.started_at DESC",self.selection());
        let rows = sqlx::query_as::<_, TripMapRow>(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(self.vehicle)
            .bind(self.from)
            .bind(self.to)
            .bind(self.search.as_deref())
            .bind(self.user)
            .bind(&self.tags.tag_ids)
            .bind(self.tags.match_all)
            .bind(self.tags.untagged)
            .fetch_all(pool)
            .await?;
        let total_trips = rows.len();
        let routes = super::map_projection::RouteAssembly::new(rows)
            .resolve(pool)
            .await?;
        let missing_route_count = total_trips.saturating_sub(routes.len());
        debug!(vehicle_id=%self.vehicle,total_trips,route_count=routes.len(),missing_route_count,"trips.map_data");
        Ok(TripMapResponse {
            vehicle_id: self.vehicle,
            from: self.from,
            to: self.to,
            total_trips,
            missing_route_count,
            routes,
        })
    }

    pub async fn timeline(
        self,
        pool: &sqlx::PgPool,
    ) -> Result<TirePressureTimelineResponse, AppError> {
        let samples=sqlx::query_as::<_,TirePressureTimelineSample>(
            "SELECT ts,tire_fl_psi,tire_fr_psi,tire_rl_psi,tire_rr_psi FROM timeseries.telemetry WHERE vehicle_id=$1 AND ts >= $2 AND ts <= $3 ORDER BY ts")
            .bind(self.vehicle).bind(self.from).bind(self.to).fetch_all(pool).await?;
        let sql = format!(
            r#"SELECT t.id,t.vehicle_id,t.started_at,t.ended_at,t.duration_seconds,
            COALESCE(t.duration_seconds::float8/60.0,EXTRACT(EPOCH FROM (t.ended_at-t.started_at))/60.0) AS duration_min,
            t.distance_miles,
            COALESCE(sg.name,NULLIF(CONCAT_WS(', ',sa.road,sa.city),'')) AS start_place,
            COALESCE(eg.name,NULLIF(CONCAT_WS(', ',ea.road,ea.city),'')) AS end_place,
            sa.display_name AS start_address,ea.display_name AS end_address,{TAG_COLUMN}
            {} WHERE t.vehicle_id=$1 AND t.started_at <= $3 AND t.ended_at >= $2 {} ORDER BY t.started_at"#,
            relations(4),
            sql_predicate("t", 5, 6, 7)
        );
        let trips = sqlx::query_as(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(self.vehicle)
            .bind(self.from)
            .bind(self.to)
            .bind(self.user)
            .bind(self.tags.tag_ids)
            .bind(self.tags.match_all)
            .bind(self.tags.untagged)
            .fetch_all(pool)
            .await?;
        Ok(TirePressureTimelineResponse {
            vehicle_id: self.vehicle,
            from: self.from,
            to: self.to,
            samples,
            trips,
        })
    }
}
