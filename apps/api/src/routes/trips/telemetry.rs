use super::*;
use sqlx::{postgres::PgRow, FromRow, Postgres, QueryBuilder};

pub(super) struct Window {
    vehicle: Uuid,
    trip: Uuid,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub duration_seconds: Option<i32>,
}

pub(super) enum Projection {
    TrackRaw,
    TrackAggregate,
    Speed,
    Elevation,
    Power,
    Series,
    Detail(i32),
    PowerEvidence,
}

impl Window {
    pub async fn load(
        pool: &sqlx::PgPool,
        vehicle: Uuid,
        trip: Uuid,
        stored_duration: bool,
    ) -> Result<Self, AppError> {
        let column = if stored_duration {
            "duration_seconds"
        } else {
            "NULL::int4 AS duration_seconds"
        };
        let sql = format!(
            "SELECT started_at,ended_at,{column} FROM panda.trips WHERE id=$1 AND vehicle_id=$2"
        );
        let row = sqlx::query_as::<_, TripWindowRow>(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(trip)
            .bind(vehicle)
            .fetch_optional(pool)
            .await?
            .ok_or(AppError::NotFound)?;
        Ok(Self {
            vehicle,
            trip,
            started_at: row.started_at,
            ended_at: row.ended_at,
            duration_seconds: row.duration_seconds,
        })
    }

    pub fn new(
        vehicle: Uuid,
        trip: Uuid,
        started_at: DateTime<Utc>,
        ended_at: DateTime<Utc>,
    ) -> Self {
        Self {
            vehicle,
            trip,
            started_at,
            ended_at,
            duration_seconds: None,
        }
    }

    pub fn track(&self) -> (&'static str, Projection) {
        match self.duration_seconds.unwrap_or(0) / 60 {
            ..15 => ("1 second", Projection::TrackRaw),
            15..60 => ("5 seconds", Projection::TrackAggregate),
            _ => ("15 seconds", Projection::TrackAggregate),
        }
    }

    pub async fn fetch<T>(
        &self,
        pool: &sqlx::PgPool,
        projection: Projection,
    ) -> Result<Vec<T>, AppError>
    where
        for<'r> T: FromRow<'r, PgRow> + Send + Unpin,
    {
        let mut sql = QueryBuilder::<Postgres>::new("SELECT ");
        let grouped = !matches!(projection, Projection::TrackRaw | Projection::PowerEvidence);
        let linked = matches!(
            projection,
            Projection::Speed
                | Projection::Power
                | Projection::Series
                | Projection::Detail(_)
                | Projection::PowerEvidence
        );
        match projection {
            Projection::TrackRaw => {
                sql.push("ts,latitude AS lat,longitude AS lng,speed_mph,altitude_m");
            }
            Projection::TrackAggregate => {
                sql.push("time_bucket('15 seconds'::interval,ts) AS ts,avg(latitude) AS lat,avg(longitude) AS lng,avg(speed_mph) AS speed_mph,avg(altitude_m) AS altitude_m");
            }
            Projection::Speed => {
                sql.push("EXTRACT(EPOCH FROM (time_bucket('10 seconds'::interval,ts)-")
                    .push_bind(self.started_at)
                    .push("))::float8 AS elapsed_s,avg(speed_mph) AS speed_mph");
            }
            Projection::Elevation => {
                sql.push("time_bucket('10 seconds'::interval,ts) AS ts,avg(altitude_m) AS value");
            }
            Projection::Power => {
                sql.push("time_bucket('10 seconds'::interval,ts) AS ts,avg(power_kw) AS power_kw,avg(regen_power_kw) AS regen_power_kw,avg(speed_mph) AS speed_mph,avg(battery_level) AS battery_level");
            }
            Projection::Series => {
                sql.push("time_bucket('10 seconds'::interval,ts) AS ts,")
                    .push(SERIES_COLUMNS);
            }
            Projection::Detail(seconds) => {
                let bucket = format!("time_bucket(make_interval(secs => {seconds}),ts)");
                sql.push(format!("{bucket} AS bucket_ts,EXTRACT(EPOCH FROM ({bucket}-"))
                    .push_bind(self.started_at).push("))::int AS elapsed_s,")
                    .push("last(latitude,ts) FILTER (WHERE latitude IS NOT NULL AND latitude<>0) AS lat,last(longitude,ts) FILTER (WHERE longitude IS NOT NULL AND longitude<>0) AS lng,last(altitude_m,ts) AS altitude_m,")
                    .push(SERIES_COLUMNS);
            }
            Projection::PowerEvidence => {
                sql.push("ts,battery_level,battery_capacity_wh,power_kw,regen_power_kw");
            }
        }
        sql.push(" FROM timeseries.telemetry WHERE vehicle_id=")
            .push_bind(self.vehicle)
            .push(" AND ts>=")
            .push_bind(self.started_at)
            .push(" AND ts<=")
            .push_bind(self.ended_at);
        if linked {
            sql.push(" AND (trip_id=")
                .push_bind(self.trip)
                .push(" OR trip_id IS NULL)");
        }
        match projection {
            Projection::TrackRaw | Projection::TrackAggregate => {
                sql.push(" AND latitude IS NOT NULL AND longitude IS NOT NULL AND NOT (latitude=0 AND longitude=0)");
            }
            Projection::Speed => {
                sql.push(" AND speed_mph IS NOT NULL");
            }
            Projection::Elevation => {
                sql.push(" AND altitude_m IS NOT NULL");
            }
            _ => (),
        }
        if grouped {
            sql.push(" GROUP BY 1");
        }
        sql.push(" ORDER BY 1");
        match projection {
            Projection::TrackRaw | Projection::TrackAggregate => {
                sql.push(" LIMIT 5000");
            }
            Projection::Detail(_) => {
                sql.push(" LIMIT 2000");
            }
            _ => (),
        }
        Ok(sql.build_query_as::<T>().fetch_all(pool).await?)
    }
}

const SERIES_COLUMNS: &str = "avg(speed_mph) AS speed_mph,avg(power_kw) AS power_kw,avg(regen_power_kw) AS regen_power_kw,avg(battery_level) AS battery_level,avg(outside_temp_c) AS outside_temp_c,avg(cabin_temp_c) AS cabin_temp_c,avg(driver_temp_c) AS driver_temp_c,bool_or(hvac_active) AS hvac_active,avg(tire_fl_psi) AS tire_fl_psi,avg(tire_fr_psi) AS tire_fr_psi,avg(tire_rl_psi) AS tire_rl_psi,avg(tire_rr_psi) AS tire_rr_psi";
