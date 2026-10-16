//! Parameterized writes with explicit per-column conflict policies.
use super::telemetry_fields::telemetry_fields;
use super::*;
use sqlx::{Postgres, QueryBuilder};

enum Merge {
    Arrival,
    Sensor(&'static str),
    Clock,
    LatestDrive,
    RawDrive,
    FirstAssociation,
}

fn assignment(table: &'static str, field: &'static str, merge: Merge) -> String {
    let incoming = format!("EXCLUDED.{field}");
    let existing = format!("{table}.{field}");
    let value = match merge {
        Merge::Arrival => format!("COALESCE({incoming}, {existing})"),
        Merge::FirstAssociation => format!("COALESCE({existing}, {incoming})"),
        Merge::Clock => format!("GREATEST({incoming}, {existing})"),
        Merge::LatestDrive => format!("COALESCE(NULLIF({incoming}, 'unknown'), {existing})"),
        Merge::RawDrive => format!(
            "CASE WHEN {incoming} IS NOT NULL AND {incoming} <> 'unknown' \
             THEN {incoming} ELSE COALESCE({existing}, {incoming}) END"
        ),
        Merge::Sensor(clock) => format!(
            "CASE WHEN EXCLUDED.{clock} IS NOT NULL \
             AND ({table}.{clock} IS NULL OR EXCLUDED.{clock} >= {table}.{clock}) \
             THEN COALESCE({incoming}, {existing}) ELSE {existing} END"
        ),
    };
    format!("{field} = {value}")
}

macro_rules! bind_value {
    ($query:ident, $event:ident, $field:ident, scalar) => {
        $query.push(", ").push_bind($event.$field);
    };
    ($query:ident, $event:ident, $field:ident, text) => {
        $query.push(", ").push_bind($event.$field.as_deref());
    };
    ($query:ident, $event:ident, $field:ident, debug) => {
        $query.push(", ").push_bind(
            $event
                .$field
                .as_ref()
                .map(|value| format!("{value:?}").to_lowercase()),
        );
    };
    ($query:ident, $event:ident, $field:ident, drive) => {
        $query
            .push(", ")
            .push_bind($event.$field.as_ref().map(|value| value.as_str()));
    };
}

macro_rules! raw_column {
    ($columns:ident, $updates:ident, $field:ident, clock) => {};
    ($columns:ident, $updates:ident, $field:ident, payload) => {
        $columns.push(stringify!($field));
        $updates.push(assignment(
            "timeseries.telemetry",
            stringify!($field),
            if stringify!($field) == "drive_mode" {
                Merge::RawDrive
            } else {
                Merge::Arrival
            },
        ));
    };
}

macro_rules! raw_value {
    ($query:ident, $event:ident, $field:ident, $kind:ident, clock) => {};
    ($query:ident, $event:ident, $field:ident, $kind:ident, payload) => {
        bind_value!($query, $event, $field, $kind);
    };
}

macro_rules! latest_merge {
    ((arrival)) => {
        Merge::Arrival
    };
    ((timestamp)) => {
        Merge::Clock
    };
    ((drive)) => {
        Merge::LatestDrive
    };
    ((fresh $clock:ident)) => {
        Merge::Sensor(stringify!($clock))
    };
}

macro_rules! latest_column {
    ($columns:ident, $updates:ident, $field:ident, (skip)) => {};
    ($columns:ident, $updates:ident, $field:ident, $policy:tt) => {
        $columns.push(stringify!($field));
        $updates.push(assignment(
            "panda.vehicle_latest_status",
            stringify!($field),
            latest_merge!($policy),
        ));
    };
}

macro_rules! latest_value {
    ($query:ident, $event:ident, $field:ident, $kind:ident, (skip)) => {};
    ($query:ident, $event:ident, $field:ident, $kind:ident, $policy:tt) => {
        bind_value!($query, $event, $field, $kind);
    };
}

macro_rules! define_writers {
    ($($field:ident: $kind:ident, $role:ident, $change:ident, $latest:tt, $wire:tt;)*) => {
        pub(super) async fn write_telemetry(
            pool: &PgPool, event: &TelemetryEvent, trip_id: Option<Uuid>, charge_session_id: Option<Uuid>,
        ) -> anyhow::Result<()> {
            let mut columns = vec!["vehicle_id", "ts"];
            let mut updates = Vec::new();
            $(raw_column!(columns, updates, $field, $role);)*
            columns.extend(["trip_id", "charge_session_id"]);
            updates.push(assignment("timeseries.telemetry", "trip_id", Merge::FirstAssociation));
            updates.push(assignment("timeseries.telemetry", "charge_session_id", Merge::Arrival));
            let mut query = QueryBuilder::<Postgres>::new(format!(
                "INSERT INTO timeseries.telemetry ({}) VALUES (", columns.join(", ")
            ));
            query.push_bind(event.vehicle_id).push(", ").push_bind(event.ts);
            $(raw_value!(query, event, $field, $kind, $role);)*
            query.push(", ").push_bind(trip_id).push(", ").push_bind(charge_session_id);
            query.push(") ON CONFLICT (vehicle_id, ts) DO UPDATE SET ").push(updates.join(", "));
            query.build().execute(pool).await?;
            Ok(())
        }

        pub(super) async fn upsert_latest_status(pool: &PgPool, event: &TelemetryEvent) -> anyhow::Result<()> {
            let mut columns = vec!["vehicle_id", "ts"];
            let mut updates = vec![assignment("panda.vehicle_latest_status", "ts", Merge::Clock)];
            $(latest_column!(columns, updates, $field, $latest);)*
            columns.push("updated_at");
            updates.push("updated_at = now()".to_owned());
            let mut query = QueryBuilder::<Postgres>::new(format!(
                "INSERT INTO panda.vehicle_latest_status ({}) VALUES (", columns.join(", ")
            ));
            query.push_bind(event.vehicle_id).push(", ").push_bind(event.ts);
            $(latest_value!(query, event, $field, $kind, $latest);)*
            query.push(", now()) ON CONFLICT (vehicle_id) DO UPDATE SET ").push(updates.join(", "));
            query.build().execute(pool).await?;
            Ok(())
        }
    };
}

telemetry_fields!(define_writers);
