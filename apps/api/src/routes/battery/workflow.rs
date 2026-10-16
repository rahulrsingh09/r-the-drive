use super::*;

pub(super) enum Signal {
    Soc,
    Range,
    Capacity,
}

pub(super) struct Window {
    vehicle: Uuid,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
}

impl Window {
    pub(super) fn new(vehicle: Uuid, params: &TimeRangeParams, days: i64) -> Self {
        let (from, to) = resolve_time_bounds(
            params.from,
            params.to,
            params.lifetime.unwrap_or(false),
            days,
        );
        Self { vehicle, from, to }
    }

    pub(super) async fn series(
        &self,
        pool: &sqlx::PgPool,
        signal: Signal,
    ) -> Result<Vec<TimeSeriesPoint>, AppError> {
        let column = match signal {
            Signal::Soc => "battery_level",
            Signal::Range => "distance_to_empty_mi",
            Signal::Capacity => "battery_capacity_wh",
        };
        let sql = format!(
            "SELECT ts, {column} AS value FROM timeseries.telemetry
            WHERE vehicle_id=$1 AND ts >= $2 AND ts <= $3 AND {column} IS NOT NULL ORDER BY ts"
        );
        Ok(sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(self.vehicle)
            .bind(self.from)
            .bind(self.to)
            .fetch_all(pool)
            .await?)
    }

    async fn capacity_samples(
        &self,
        pool: &sqlx::PgPool,
    ) -> Result<Vec<BatteryMileageSampleRow>, AppError> {
        Ok(sqlx::query_as("SELECT ts AS bucket, odometer_miles, battery_capacity_wh,
            distance_to_empty_mi, battery_level FROM timeseries.telemetry
            WHERE vehicle_id=$1 AND ts >= $2 AND ts <= $3 AND battery_capacity_wh > 10000 ORDER BY ts")
            .bind(self.vehicle).bind(self.from).bind(self.to).fetch_all(pool).await?)
    }

    pub(super) async fn degradation(
        &self,
        pool: &sqlx::PgPool,
    ) -> Result<Vec<DegradationPoint>, AppError> {
        let reference = CapacityReference::load(pool, self.vehicle).await?;
        Ok(self
            .capacity_samples(pool)
            .await?
            .into_iter()
            .map(|sample| {
                let wh = sample.battery_capacity_wh.unwrap_or(0.0);
                DegradationPoint {
                    ts: sample.bucket,
                    odometer_mi: sample.odometer_miles,
                    usable_kwh: wh / 1000.0,
                    rated_kwh: None,
                    capacity_pct: reference
                        .wh()
                        .filter(|value| *value > 0.0)
                        .map(|baseline| wh / baseline * 100.0),
                }
            })
            .collect())
    }

    pub(super) async fn mileage(
        &self,
        pool: &sqlx::PgPool,
    ) -> Result<Vec<BatteryMileagePoint>, AppError> {
        let reference = CapacityReference::load(pool, self.vehicle).await?;
        let model: Option<String> =
            sqlx::query_scalar("SELECT model FROM panda.vehicles WHERE id=$1")
                .bind(self.vehicle)
                .fetch_optional(pool)
                .await?;
        let projection = MileageProjection {
            baseline_wh: reference.wh().unwrap_or(0.0),
            model,
        };
        Ok(self
            .capacity_samples(pool)
            .await?
            .into_iter()
            .map(|sample| projection.project(sample))
            .collect())
    }

    pub(super) async fn phantom(
        &self,
        pool: &sqlx::PgPool,
    ) -> Result<Vec<PhantomDrainPoint>, AppError> {
        let periods =
            fetch_validated_idle_drain_periods_for_chart(pool, self.vehicle, self.from, self.to)
                .await?;
        let days = periods.into_iter().fold(
            BTreeMap::<NaiveDate, ParkedDay>::new(),
            |mut days, period| {
                if let Some(start) = period.period_start {
                    let day = days.entry(start.date_naive()).or_default();
                    day.loss += period.soc_lost_pct.unwrap_or(0.0);
                    day.hours += period.duration_hours.unwrap_or(0.0);
                }
                days
            },
        );
        Ok(days
            .into_iter()
            .map(|(day, total)| PhantomDrainPoint {
                day: Some(day),
                total_soc_lost: Some(total.loss),
                hours_parked: Some(total.hours),
                avg_drain_rate: (total.hours > 0.0).then_some(total.loss / total.hours),
            })
            .collect())
    }
}

#[derive(Default)]
struct ParkedDay {
    loss: f64,
    hours: f64,
}

pub(super) enum CapacityReference {
    Stored(f64),
    Observed(Option<f64>),
}

impl CapacityReference {
    pub(super) async fn load(pool: &sqlx::PgPool, vehicle: Uuid) -> Result<Self, AppError> {
        let stored: Option<f64> =
            sqlx::query_scalar("SELECT battery_capacity_wh FROM panda.vehicles WHERE id=$1")
                .bind(vehicle)
                .fetch_optional(pool)
                .await?
                .flatten();
        match stored.filter(|value| *value > 10000.0) {
            Some(value) => Ok(Self::Stored(value)),
            None => Ok(Self::Observed(
                sqlx::query_scalar(
                    "SELECT max(battery_capacity_wh) FROM timeseries.telemetry
                WHERE vehicle_id=$1 AND battery_capacity_wh > 10000",
                )
                .bind(vehicle)
                .fetch_optional(pool)
                .await?
                .flatten(),
            )),
        }
    }

    pub(super) fn wh(&self) -> Option<f64> {
        match self {
            Self::Stored(value) => Some(*value),
            Self::Observed(value) => *value,
        }
    }
}

struct MileageProjection {
    baseline_wh: f64,
    model: Option<String>,
}

impl MileageProjection {
    fn project(&self, sample: BatteryMileageSampleRow) -> BatteryMileagePoint {
        let usable_kwh = sample
            .battery_capacity_wh
            .filter(|value| value.is_finite() && *value > 0.0)
            .map(|wh| if wh > 1000.0 { wh / 1000.0 } else { wh });
        let range_mi = normalize_remaining_range_miles_strict(
            sample.distance_to_empty_mi,
            sample.battery_level,
            sample.battery_capacity_wh,
            self.model.as_deref(),
        );
        BatteryMileagePoint {
            ts: sample.bucket,
            odometer_mi: sample.odometer_miles.filter(|value| value.is_finite()),
            usable_kwh,
            range_mi,
            projected_max_range_mi: projected_full_charge_range_miles(
                range_mi,
                sample.battery_level,
            ),
            degradation_pct: usable_kwh
                .filter(|_| self.baseline_wh > 0.0)
                .map(|kwh| (100.0 - (kwh * 1000.0 / self.baseline_wh * 100.0)).max(0.0)),
        }
    }
}

#[derive(sqlx::FromRow)]
struct ChargingEvidence {
    charge_count: Option<i64>,
    total_added_kwh: Option<f64>,
    total_used_kwh: Option<f64>,
}

pub(super) struct HealthEvidence {
    reference: CapacityReference,
    daily_peak_average_wh: Option<f64>,
    charging: ChargingEvidence,
}

impl HealthEvidence {
    pub(super) async fn load(pool: &sqlx::PgPool, vehicle: Uuid) -> Result<Self, AppError> {
        let reference = CapacityReference::load(pool, vehicle).await?;
        let daily_peak_average_wh = sqlx::query_scalar(
            "WITH daily AS (
            SELECT date_trunc('day',ts) AS day, max(battery_capacity_wh) AS peak
            FROM timeseries.telemetry WHERE vehicle_id=$1 AND battery_capacity_wh > 10000
                AND ts >= now() - INTERVAL '30 days' GROUP BY date_trunc('day',ts)
            ) SELECT avg(peak) FROM daily",
        )
        .bind(vehicle)
        .fetch_optional(pool)
        .await?
        .flatten();
        let charging = sqlx::query_as(
            "WITH eligible AS (
            SELECT COALESCE(kwh_added, energy_added_wh / 1000.0) AS added,
                COALESCE(energy_used_wh / 1000.0, 0) AS used FROM panda.charge_sessions
            WHERE vehicle_id=$1 AND COALESCE(kwh_added, energy_added_wh / 1000.0) > 0.01
            ) SELECT COUNT(*) AS charge_count, SUM(added) AS total_added_kwh,
                SUM(GREATEST(added, used)) AS total_used_kwh FROM eligible",
        )
        .bind(vehicle)
        .fetch_one(pool)
        .await?;
        Ok(Self {
            reference,
            daily_peak_average_wh,
            charging,
        })
    }

    pub(super) fn project(self) -> serde_json::Value {
        let now = self.daily_peak_average_wh.map(|wh| wh / 1000.0);
        let new = self.reference.wh().map(|wh| wh / 1000.0);
        let health = now
            .zip(new)
            .filter(|(_, new)| *new > 0.0)
            .map(|(now, new)| (now / new * 100.0).min(100.0));
        let cycles = self
            .charging
            .total_added_kwh
            .zip(new)
            .filter(|(_, new)| *new > 0.0)
            .map(|(added, new)| (added / new).floor());
        let efficiency = self
            .charging
            .total_added_kwh
            .zip(self.charging.total_used_kwh)
            .filter(|(_, used)| *used > 0.0)
            .map(|(added, used)| added / used * 100.0);
        serde_json::json!({
            "usable_now_kwh": now, "usable_new_kwh": new, "battery_health_pct": health,
            "estimated_degradation_pct": health.map(|pct| (100.0-pct).max(0.0)),
            "charging_cycles": cycles, "charge_count": self.charging.charge_count,
            "total_energy_added_kwh": self.charging.total_added_kwh,
            "total_energy_used_kwh": self.charging.total_used_kwh,
            "charging_efficiency_pct": efficiency,
        })
    }
}
