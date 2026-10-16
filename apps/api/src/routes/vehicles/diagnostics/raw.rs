use super::*;
use sqlx::{
    postgres::{PgArguments, PgRow},
    FromRow, Postgres,
};

pub(super) struct RawPlan {
    fields: Vec<&'static str>,
    scope: ReadScope,
    page: Page,
    selection: String,
}

struct Page {
    number: i64,
    size: i64,
    offset: i64,
}
impl Page {
    fn new(params: &RawDataParams) -> Self {
        let size = params.per_page.or(params.limit).unwrap_or(25).clamp(1, 200);
        let number = params.page.unwrap_or(1).max(1);
        Self {
            number,
            size,
            offset: (number - 1).saturating_mul(size),
        }
    }
}

struct ReadScope {
    from: Option<chrono::DateTime<Utc>>,
    to: Option<chrono::DateTime<Utc>>,
    search: Option<String>,
}
impl ReadScope {
    fn query<'a, T>(
        &'a self,
        vehicle: Uuid,
        sql: &'a str,
    ) -> sqlx::query::QueryAs<'a, Postgres, T, PgArguments>
    where
        T: for<'r> FromRow<'r, PgRow>,
    {
        sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(vehicle)
            .bind(self.from)
            .bind(self.to)
            .bind(self.search.as_deref())
    }
}

enum Population<'a> {
    AnyRow,
    AnyValue(&'a [&'a str]),
}
impl<'a> Population<'a> {
    fn select(fields: &'a [&'a str], populated_only: bool) -> Self {
        match (fields.is_empty(), populated_only) {
            (true, false) => Self::AnyRow,
            (true, true) => Self::AnyValue(RAW_TELEMETRY_FIELDS),
            (false, _) => Self::AnyValue(fields),
        }
    }

    fn predicate(self) -> String {
        let mut predicate = String::from("t.vehicle_id = $1 AND ($2::timestamptz IS NULL OR t.ts >= $2) AND ($3::timestamptz IS NULL OR t.ts <= $3) AND ($4::text IS NULL OR to_jsonb(t)::text ILIKE '%' || $4 || '%')");
        if let Self::AnyValue(fields) = self {
            let alternatives: Vec<_> = fields
                .iter()
                .map(|field| format!("t.{field} IS NOT NULL"))
                .collect();
            predicate.push_str(" AND (");
            predicate.push_str(&alternatives.join(" OR "));
            predicate.push(')');
        }
        predicate
    }
}

pub(super) fn select_fields(input: Option<&str>) -> Result<Vec<&'static str>, AppError> {
    let text = input.unwrap_or_default().trim();
    if text.len() > 2048 || text.chars().any(char::is_control) {
        return Err(AppError::Validation(
            "telemetry field selector is invalid".into(),
        ));
    }
    let mut used = [false; RAW_TELEMETRY_FIELDS.len()];
    let mut fields = Vec::new();
    for token in text
        .split(',')
        .map(str::trim)
        .filter(|token| !token.is_empty())
    {
        let index = RAW_TELEMETRY_FIELDS
            .iter()
            .position(|name| *name == token)
            .ok_or_else(|| AppError::Validation(format!("unknown telemetry field: {token}")))?;
        if !used[index] {
            fields.push(RAW_TELEMETRY_FIELDS[index]);
            used[index] = true;
        }
    }
    Ok(fields)
}

pub(super) fn predicate(fields: &[&str], populated_only: bool) -> String {
    Population::select(fields, populated_only).predicate()
}

const COVERAGE: &[(&str, &str)] = &[
    ("first_event_at", "min(ts)"),
    ("last_event_at", "max(ts)"),
    ("sample_count", "count(*)"),
    ("odometer_samples", "count(odometer_miles)"),
    ("battery_samples", "count(battery_level)"),
    ("range_samples", "count(distance_to_empty_mi)"),
    ("outside_temp_samples", "count(outside_temp_c)"),
    ("power_samples", "count(power_kw)"),
    ("regen_samples", "count(regen_power_kw)"),
    (
        "tire_pressure_samples",
        "greatest(count(tire_fl_psi), count(tire_fl_status))",
    ),
    ("lock_samples", "count(door_front_left_locked)"),
    ("software_samples", "count(ota_status)"),
];

pub(super) fn field_coverage_query(selection: &str) -> String {
    let (names, counts): (Vec<_>, Vec<_>) = RAW_TELEMETRY_FIELDS
        .iter()
        .map(|field| (format!("'{field}'"), format!("COUNT(t.{field})")))
        .unzip();
    format!("SELECT unnest(ARRAY[{}]::text[]) AS field, unnest(ARRAY[{}]::bigint[]) AS sample_count FROM timeseries.telemetry t WHERE {selection}", names.join(", "), counts.join(", "))
}

impl RawPlan {
    pub(super) fn compile(params: RawDataParams) -> Result<Self, AppError> {
        validate_raw_time_bounds(params.from, params.to)?;
        let fields = parse_raw_fields(params.fields.as_deref())?;
        let selection = raw_telemetry_where_clause(&fields, params.populated_only.unwrap_or(false));
        let page = Page::new(&params);
        let scope = ReadScope {
            from: params.from,
            to: params.to,
            search: params
                .search
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_owned),
        };
        Ok(Self {
            fields,
            scope,
            page,
            selection,
        })
    }

    pub(super) async fn read(
        self,
        pool: &sqlx::PgPool,
        vehicle: Uuid,
    ) -> Result<serde_json::Value, AppError> {
        let samples_sql = format!("SELECT ts, {} FROM timeseries.telemetry t WHERE {} ORDER BY t.ts DESC LIMIT $5 OFFSET $6", RAW_TELEMETRY_FIELDS.join(", "), self.selection);
        let samples: Vec<RawVehicleSampleRow> = self
            .scope
            .query(vehicle, &samples_sql)
            .bind(self.page.size)
            .bind(self.page.offset)
            .fetch_all(pool)
            .await?;
        let coverage_sql = format!(
            "SELECT {} FROM timeseries.telemetry t WHERE {}",
            COVERAGE
                .iter()
                .map(|(name, expression)| format!("{expression} AS {name}"))
                .collect::<Vec<_>>()
                .join(", "),
            self.selection
        );
        // Keep the four read-committed observations and their failure order independent.
        let coverage: RawVehicleCoverageRow = self
            .scope
            .query(vehicle, &coverage_sql)
            .fetch_one(pool)
            .await?;
        let count_sql = format!(
            "SELECT COUNT(*)::BIGINT FROM timeseries.telemetry t WHERE {}",
            self.selection
        );
        let (total,): (i64,) = self
            .scope
            .query(vehicle, &count_sql)
            .fetch_one(pool)
            .await?;
        let field_sql = raw_field_coverage_query(&self.selection);
        let fields: Vec<RawTelemetryFieldCoverageRow> = self
            .scope
            .query(vehicle, &field_sql)
            .fetch_all(pool)
            .await?;
        Ok(RawResult {
            samples,
            coverage,
            total,
            fields,
        }
        .project(vehicle, self.page, self.fields))
    }
}

struct RawResult {
    samples: Vec<RawVehicleSampleRow>,
    coverage: RawVehicleCoverageRow,
    total: i64,
    fields: Vec<RawTelemetryFieldCoverageRow>,
}
impl RawResult {
    fn project(self, vehicle: Uuid, page: Page, selected: Vec<&str>) -> serde_json::Value {
        let coverage = self.coverage;
        serde_json::json!({
            "vehicle_id": vehicle,
            "coverage": {
                "first_event_at": coverage.first_event_at, "last_event_at": coverage.last_event_at,
                "sample_count": coverage.sample_count, "odometer_samples": coverage.odometer_samples,
                "battery_samples": coverage.battery_samples, "range_samples": coverage.range_samples,
                "outside_temp_samples": coverage.outside_temp_samples, "power_samples": coverage.power_samples,
                "regen_samples": coverage.regen_samples, "tire_pressure_samples": coverage.tire_pressure_samples,
                "lock_samples": coverage.lock_samples, "software_samples": coverage.software_samples
            },
            "samples": self.samples.into_iter().map(raw_sample_json).collect::<Vec<_>>(),
            "total": self.total, "limit": page.size, "per_page": page.size,
            "offset": page.offset, "page": page.number,
            "selected_fields": selected, "field_coverage": self.fields
        })
    }
}

pub(super) fn project_sample(row: RawVehicleSampleRow) -> serde_json::Value {
    macro_rules! columns {
        ($($field:ident),+ $(,)?) => {{
            let RawVehicleSampleRow { ts, $($field),+ } = row;
            let values = [$((stringify!($field), serde_json::json!($field))),+];
            let mut sample: serde_json::Map<String,serde_json::Value> = values.into_iter().map(|(name,value)| (name.into(),value)).collect();
            sample.insert("ts".into(),serde_json::json!(ts));
            serde_json::Value::Object(sample)
        }};
    }
    columns!(
        latitude,
        longitude,
        altitude_m,
        speed_mph,
        battery_level,
        battery_capacity_wh,
        distance_to_empty_mi,
        battery_limit,
        power_state,
        charger_state,
        charger_status,
        time_to_end_of_charge_min,
        drive_mode,
        gear_status,
        cabin_temp_c,
        driver_temp_c,
        outside_temp_c,
        hvac_active,
        power_kw,
        regen_power_kw,
        heading_deg,
        odometer_miles,
        tire_fl_psi,
        tire_fr_psi,
        tire_rl_psi,
        tire_rr_psi,
        tire_fl_status,
        tire_fr_status,
        tire_rl_status,
        tire_rr_status,
        tire_fl_valid,
        tire_fr_valid,
        tire_rl_valid,
        tire_rr_valid,
        door_front_left_locked,
        door_front_right_locked,
        door_rear_left_locked,
        door_rear_right_locked,
        door_front_left_closed,
        door_front_right_closed,
        door_rear_left_closed,
        door_rear_right_closed,
        closure_frunk_closed,
        closure_liftgate_closed,
        closure_tailgate_closed,
        ota_current_version,
        ota_available_version,
        ota_status,
        ota_current_status,
        hv_thermal_event,
        twelve_volt_health,
        is_online
    )
}
