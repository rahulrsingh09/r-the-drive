use super::*;

type Field = (&'static str, fn(&TelemetryLaneRow) -> Option<f64>);

macro_rules! columns {
    ($($constant:ident => $field:ident),+ $(,)?) => {
        $(const $constant: Field = (stringify!($field), |row| row.$field);)+
        const FIELDS: &[Field] = &[$($constant),+];
    };
}
columns!(
    LATITUDE => latitude,
    LONGITUDE => longitude,
    ALTITUDE_M => altitude_m,
    SPEED_MPH => speed_mph,
    BATTERY_LEVEL => battery_level,
    BATTERY_CAPACITY_WH => battery_capacity_wh,
    DISTANCE_TO_EMPTY_MI => distance_to_empty_mi,
    BATTERY_LIMIT => battery_limit,
    TIME_TO_END_OF_CHARGE_MIN => time_to_end_of_charge_min,
    CABIN_TEMP_C => cabin_temp_c,
    DRIVER_TEMP_C => driver_temp_c,
    OUTSIDE_TEMP_C => outside_temp_c,
    POWER_KW => power_kw,
    REGEN_POWER_KW => regen_power_kw,
    HEADING_DEG => heading_deg,
    ODOMETER_MILES => odometer_miles,
    TIRE_FL_PSI => tire_fl_psi,
    TIRE_FR_PSI => tire_fr_psi,
    TIRE_RL_PSI => tire_rl_psi,
    TIRE_RR_PSI => tire_rr_psi
);

struct LaneSpec {
    name: &'static str,
    fields: &'static [Field],
}
const SPECS: &[LaneSpec] = &[
    LaneSpec {
        name: "battery",
        fields: &[
            BATTERY_LEVEL,
            BATTERY_CAPACITY_WH,
            DISTANCE_TO_EMPTY_MI,
            BATTERY_LIMIT,
        ],
    },
    LaneSpec {
        name: "drive",
        fields: &[SPEED_MPH, ODOMETER_MILES, POWER_KW, REGEN_POWER_KW],
    },
    LaneSpec {
        name: "location",
        fields: &[LATITUDE, LONGITUDE, ALTITUDE_M, HEADING_DEG],
    },
    LaneSpec {
        name: "climate",
        fields: &[CABIN_TEMP_C, DRIVER_TEMP_C, OUTSIDE_TEMP_C],
    },
    LaneSpec {
        name: "charging",
        fields: &[TIME_TO_END_OF_CHARGE_MIN, POWER_KW, REGEN_POWER_KW],
    },
    LaneSpec {
        name: "health",
        fields: &[TIRE_FL_PSI, TIRE_FR_PSI, TIRE_RL_PSI, TIRE_RR_PSI],
    },
];

struct Selection {
    specs: Vec<&'static LaneSpec>,
}
impl Selection {
    fn parse(input: Option<&str>) -> Result<Self, AppError> {
        let text = input.unwrap_or("battery,drive");
        if text.len() > 256 || text.chars().any(char::is_control) {
            return Err(AppError::Validation(
                "telemetry lane selector is invalid".into(),
            ));
        }
        let mut selected = [false; SPECS.len()];
        let mut specs = Vec::new();
        for token in text
            .split(',')
            .map(str::trim)
            .filter(|token| !token.is_empty())
        {
            let index = SPECS
                .iter()
                .position(|spec| spec.name == token)
                .ok_or_else(|| AppError::Validation(format!("unknown telemetry lane: {token}")))?;
            if !selected[index] {
                selected[index] = true;
                specs.push(&SPECS[index]);
            }
        }
        if !(1..=4).contains(&specs.len()) {
            return Err(AppError::Validation(
                "select between 1 and 4 telemetry lanes".into(),
            ));
        }
        Ok(Self { specs })
    }

    fn query(&self) -> String {
        // Unselected aggregates still participate in the original database failure contract.
        projection_query()
    }
}

fn projection_query() -> String {
    let fields = FIELDS
        .iter()
        .map(|(name, _)| format!("AVG(t.{name})::double precision AS {name}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("SELECT date_bin(make_interval(secs => $4::double precision), t.ts, $2) AS bucket, {fields} FROM timeseries.telemetry t WHERE t.vehicle_id = $1 AND t.ts >= $2 AND t.ts <= $3 GROUP BY bucket ORDER BY bucket LIMIT $5")
}

#[cfg(test)]
pub(super) fn names(input: Option<&str>) -> Result<Vec<&'static str>, AppError> {
    Ok(Selection::parse(input)?
        .specs
        .iter()
        .map(|spec| spec.name)
        .collect())
}

pub(super) fn resolution(
    input: Option<&str>,
    from: chrono::DateTime<Utc>,
    to: chrono::DateTime<Utc>,
    points: i64,
) -> Result<i64, AppError> {
    let requested = input.unwrap_or("auto");
    let fixed = [("1m", 60), ("5m", 300), ("1h", 3600)];
    if let Some((_, seconds)) = fixed.iter().find(|(name, _)| *name == requested) {
        return Ok(*seconds);
    }
    if requested != "auto" {
        return Err(AppError::Validation(format!(
            "unsupported telemetry resolution: {requested}"
        )));
    }
    Ok(1 + ((to - from).num_seconds().max(1) - 1) / points)
}

pub(super) struct LanePlan {
    selection: Selection,
    window: TelemetryLaneWindow,
    points: i64,
}
impl LanePlan {
    pub(super) fn compile(params: TelemetryLaneParams) -> Result<Self, AppError> {
        let selection = Selection::parse(params.lanes.as_deref())?;
        let to = params.to.unwrap_or_else(Utc::now);
        let from = params
            .from
            .unwrap_or_else(|| to - chrono::Duration::hours(24));
        validate_raw_time_bounds(Some(from), Some(to))?;
        if to - from > chrono::Duration::days(90) {
            return Err(AppError::Validation(
                "telemetry lane windows cannot exceed 90 days".into(),
            ));
        }
        let points = params.max_points.unwrap_or(256).clamp(64, 512);
        let seconds = resolution(params.resolution.as_deref(), from, to, points)?;
        Ok(Self {
            selection,
            points,
            window: TelemetryLaneWindow {
                from,
                to,
                resolution_seconds: seconds,
                approximate: seconds > 1,
            },
        })
    }

    pub(super) async fn read(
        self,
        pool: &sqlx::PgPool,
        vehicle: Uuid,
    ) -> Result<TelemetryLaneFrame, AppError> {
        let query = self.selection.query();
        let rows = sqlx::query_as::<_, TelemetryLaneRow>(sqlx::AssertSqlSafe(query))
            .bind(vehicle)
            .bind(self.window.from)
            .bind(self.window.to)
            .bind(self.window.resolution_seconds as f64)
            .bind(self.points)
            .fetch_all(pool)
            .await?;
        let mut accumulators: Vec<_> = self
            .selection
            .specs
            .iter()
            .map(|spec| LaneAccumulator::new(spec, rows.len()))
            .collect();
        let mut spine = Vec::with_capacity(rows.len());
        for row in &rows {
            spine.push(row.bucket);
            for accumulator in &mut accumulators {
                accumulator.append(row);
            }
        }
        let lanes = accumulators
            .into_iter()
            .map(|lane| (lane.spec.name.into(), lane.finish()))
            .collect();
        Ok(TelemetryLaneFrame {
            vehicle_id: vehicle,
            window: self.window,
            spine,
            lanes,
            truncated: rows.len() >= self.points as usize,
        })
    }
}

struct LaneAccumulator {
    spec: &'static LaneSpec,
    values: Vec<Vec<Option<f64>>>,
    counts: Vec<i64>,
}
impl LaneAccumulator {
    fn new(spec: &'static LaneSpec, capacity: usize) -> Self {
        Self {
            spec,
            values: spec
                .fields
                .iter()
                .map(|_| Vec::with_capacity(capacity))
                .collect(),
            counts: vec![0; spec.fields.len()],
        }
    }
    fn append(&mut self, row: &TelemetryLaneRow) {
        for ((field, values), count) in self
            .spec
            .fields
            .iter()
            .zip(&mut self.values)
            .zip(&mut self.counts)
        {
            let value = field.1(row);
            *count += i64::from(value.is_some());
            values.push(value);
        }
    }
    fn finish(self) -> TelemetryLane {
        let mut numeric = BTreeMap::new();
        let mut coverage = BTreeMap::new();
        for ((field, values), count) in self.spec.fields.iter().zip(self.values).zip(self.counts) {
            let name = field.0;
            numeric.insert(name.into(), values);
            coverage.insert(name.into(), count);
        }
        TelemetryLane {
            numeric,
            coverage,
            source: "bucketed_normalized_telemetry",
        }
    }
}

#[cfg(test)]
pub(super) fn full_query() -> String {
    projection_query()
}
