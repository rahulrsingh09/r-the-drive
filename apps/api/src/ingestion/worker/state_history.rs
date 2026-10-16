//! Evidence selection and durable state-period rotation are separate stages.
use super::*;

#[derive(sqlx::FromRow)]
struct PeriodRow {
    id: i64,
    state: String,
    started_at: DateTime<Utc>,
}

impl TryFrom<PeriodRow> for ActiveStatePeriod {
    type Error = anyhow::Error;

    fn try_from(row: PeriodRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.id,
            state: row.state.parse().map_err(anyhow::Error::msg)?,
            started_at: row.started_at,
        })
    }
}

#[derive(sqlx::FromRow)]
struct TailEvidence {
    ts: DateTime<Utc>,
    power_state: Option<String>,
    charger_state: Option<String>,
    ota_status: Option<String>,
    ota_current_status: Option<String>,
    is_online: Option<bool>,
}

impl TailEvidence {
    fn state(&self) -> Option<VehicleState> {
        let power = self
            .power_state
            .as_deref()
            .and_then(|state| state.parse().ok());
        let charger = self
            .charger_state
            .as_deref()
            .and_then(|state| state.parse().ok());
        let state = infer_vehicle_state_values(
            power.as_ref(),
            charger.as_ref(),
            self.ota_status.as_deref(),
            self.ota_current_status.as_deref(),
            self.is_online,
        );
        (power.is_some() || state != VehicleState::Unknown).then_some(state)
    }
}

enum Rotation {
    Retain,
    IgnoreLate,
    Replace,
}

impl Rotation {
    fn plan(current: Option<&ActiveStatePeriod>, next: &VehicleState, at: DateTime<Utc>) -> Self {
        match current {
            Some(period) if &period.state == next => Self::Retain,
            Some(period) if at < period.started_at => Self::IgnoreLate,
            _ => Self::Replace,
        }
    }
}

async fn read_open(
    connection: &mut PgConnection,
    vehicle: Uuid,
) -> anyhow::Result<Option<ActiveStatePeriod>> {
    sqlx::query_as::<_, PeriodRow>(
        "SELECT id,state,started_at FROM panda.vehicle_state_periods \
         WHERE vehicle_id=$1 AND ended_at IS NULL ORDER BY started_at DESC LIMIT 1",
    )
    .bind(vehicle)
    .fetch_optional(connection)
    .await?
    .map(TryInto::try_into)
    .transpose()
}

pub(super) async fn load_open_state_period(
    pool: &PgPool,
    vehicle: Uuid,
) -> anyhow::Result<Option<ActiveStatePeriod>> {
    read_open(&mut *pool.acquire().await?, vehicle).await
}

pub(super) async fn transition_state_period(
    pool: &PgPool,
    vehicle_id: Uuid,
    _cached: Option<ActiveStatePeriod>,
    next_state: VehicleState,
    observed_at: DateTime<Utc>,
) -> anyhow::Result<Option<ActiveStatePeriod>> {
    let mut transaction = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1), hashtext($2))")
        .bind("panda-state-period")
        .bind(vehicle_id.to_string())
        .execute(&mut *transaction)
        .await?;
    let current = read_open(&mut transaction, vehicle_id).await?;
    let plan = Rotation::plan(current.as_ref(), &next_state, observed_at);
    if !matches!(plan, Rotation::Replace) {
        if matches!(plan, Rotation::IgnoreLate) {
            tracing::debug!(vehicle_id=%vehicle_id, observed_at=%observed_at, active_started_at=%current.as_ref().expect("late period exists").started_at, "ignoring out-of-order state-period event");
        }
        transaction.commit().await?;
        return Ok(current);
    }
    if let Some(period) = current {
        sqlx::query(
            "UPDATE panda.vehicle_state_periods SET ended_at=$1 WHERE id=$2 AND ended_at IS NULL",
        )
        .bind(observed_at)
        .bind(period.id)
        .execute(&mut *transaction)
        .await?;
    }
    let inserted = sqlx::query_as::<_, PeriodRow>(
        "INSERT INTO panda.vehicle_state_periods(vehicle_id,state,started_at) VALUES($1,$2,$3) \
         ON CONFLICT(vehicle_id) WHERE ended_at IS NULL DO NOTHING RETURNING id,state,started_at",
    )
    .bind(vehicle_id)
    .bind(next_state.to_string())
    .bind(observed_at)
    .fetch_optional(&mut *transaction)
    .await?;
    transaction.commit().await?;
    match inserted {
        Some(row) => Ok(Some(row.try_into()?)),
        None => {
            tracing::warn!(vehicle_id=%vehicle_id, "state-period insert conflicted; reloading durable open period");
            load_open_state_period(pool, vehicle_id).await
        }
    }
}

pub(super) async fn reconcile_state_period_tail(
    pool: &PgPool,
    vehicle_id: Uuid,
    cached: Option<ActiveStatePeriod>,
) -> anyhow::Result<Option<ActiveStatePeriod>> {
    let evidence = sqlx::query_as::<_, TailEvidence>(
        "SELECT ts,power_state,charger_state,ota_status,ota_current_status,is_online \
         FROM timeseries.telemetry WHERE vehicle_id=$1 AND ts >= $2 \
         AND (power_state IS NOT NULL OR charger_state IS NOT NULL OR is_online IS NOT NULL \
              OR ota_status IS NOT NULL OR ota_current_status IS NOT NULL) \
         ORDER BY ts DESC LIMIT 1",
    )
    .bind(vehicle_id)
    .bind(Utc::now() - Duration::hours(STATE_PERIOD_RECONCILE_LOOKBACK_HOURS))
    .fetch_optional(pool)
    .await?;
    if let Some((at, state)) = evidence.and_then(|row| row.state().map(|state| (row.ts, state))) {
        transition_state_period(pool, vehicle_id, cached, state, at).await
    } else {
        load_open_state_period(pool, vehicle_id).await
    }
}

pub(super) fn infer_vehicle_state(event: &TelemetryEvent) -> VehicleState {
    infer_vehicle_state_values(
        event.power_state.as_ref(),
        event.charger_state.as_ref(),
        event.ota_status.as_deref(),
        event.ota_current_status.as_deref(),
        event.is_online,
    )
}

pub(super) fn infer_vehicle_state_values(
    power: Option<&PowerState>,
    charger: Option<&ChargerState>,
    ota: Option<&str>,
    ota_current: Option<&str>,
    online: Option<bool>,
) -> VehicleState {
    let installing = ota == Some("installing") || ota_current == Some("installing");
    match (charger, installing, power, online) {
        (Some(ChargerState::Charging), _, _, _) => VehicleState::Charging,
        (_, true, _, _) => VehicleState::Updating,
        (_, _, Some(PowerState::Drive | PowerState::Go), _) => VehicleState::Drive,
        (_, _, Some(PowerState::Sleep), _) => VehicleState::Sleep,
        (_, _, Some(PowerState::Charging), _) => VehicleState::Charging,
        (_, _, _, Some(false)) => VehicleState::Offline,
        (_, _, Some(PowerState::Ready), _) => VehicleState::Ready,
        _ => VehicleState::Unknown,
    }
}

pub(super) fn lifecycle_updates(
    event: &TelemetryEvent,
    parallax: bool,
    charge_active: bool,
) -> LifecycleUpdates {
    let source_allowed = !(parallax && charge_active && event.charger_state.is_none());
    let power = source_allowed && event.power_state.is_some();
    LifecycleUpdates {
        state_period: source_allowed
            && (power || infer_vehicle_state(event) != VehicleState::Unknown),
        power_state: power,
        charging: source_allowed
            && (power || event.charger_state.is_some() || event.charger_status.is_some()),
    }
}
