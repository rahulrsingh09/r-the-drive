//! Typed sparse patches and the browser projection share the same field inventory.
use super::telemetry_fields::telemetry_fields;
use super::*;
use serde::Serialize;
use serde_json::{json, Map, Value};

macro_rules! duplicate {
    ($event:ident, $state:ident, $field:ident, clock) => {
        true
    };
    ($event:ident, $state:ident, $field:ident, payload) => {
        $event
            .$field
            .as_ref()
            .is_none_or(|value| Some(value) == $state.$field.as_ref())
    };
}

macro_rules! present {
    ($event:ident, $field:ident, clock) => {
        false
    };
    ($event:ident, $field:ident, payload) => {
        $event.$field.is_some()
    };
}

macro_rules! immediate {
    ($event:ident, $previous:ident, $field:ident, passive) => {
        false
    };
    ($event:ident, $previous:ident, $field:ident, changed) => {
        $event.$field.is_some() && $event.$field != $previous.$field
    };
    ($event:ident, $previous:ident, $field:ident, odometer) => {
        changed_f64($event.$field, $previous.$field, 0.01)
    };
}

fn insert<T: Serialize>(data: &mut Map<String, Value>, key: &str, value: Option<T>) {
    if let Some(value) = value {
        data.insert(key.to_owned(), json!(value));
    }
}

macro_rules! project {
    ($data:ident, $event:ident, $field:ident, $kind:ident, (skip)) => {};
    ($data:ident, $event:ident, $field:ident, $kind:ident, (capacity)) => {
        insert(
            &mut $data,
            "battery_capacity_kwh",
            $event.$field.map(|wh| wh / 1000.0),
        );
    };
    ($data:ident, $event:ident, $field:ident, $kind:ident, (rename $key:ident)) => {
        insert(&mut $data, stringify!($key), $event.$field.as_ref());
    };
    ($data:ident, $event:ident, $field:ident, debug, (field)) => {
        insert(
            &mut $data,
            stringify!($field),
            $event
                .$field
                .as_ref()
                .map(|value| format!("{value:?}").to_lowercase()),
        );
    };
    ($data:ident, $event:ident, $field:ident, drive, (field)) => {
        project!($data, $event, $field, debug, (field));
    };
    ($data:ident, $event:ident, $field:ident, $kind:ident, (field)) => {
        insert(&mut $data, stringify!($field), $event.$field.as_ref());
    };
}

macro_rules! socket_present {
    ($event:ident, is_online, $role:ident) => {
        false
    };
    ($event:ident, $field:ident, $role:ident) => {
        present!($event, $field, $role)
    };
}

macro_rules! define_values {
    ($($field:ident: $kind:ident, $role:ident, $change:ident, $latest:tt, $wire:tt;)*) => {
        pub(super) fn patch_event(target: &mut TelemetryEvent, source: &TelemetryEvent) {
            target.ts = source.ts;
            $(
                if let Some(value) = source.$field.as_ref() {
                    target.$field = Some(value.clone());
                }
            )*
        }

        pub(super) fn event_is_duplicate_patch(event: &TelemetryEvent, state: &TelemetryEvent) -> bool {
            true $(&& duplicate!(event, state, $field, $role))*
        }

        pub(super) fn event_has_meaningful_payload(event: &TelemetryEvent) -> bool {
            false $(|| present!(event, $field, $role))*
        }

        pub(crate) fn socket_payload_present(event: &TelemetryEvent) -> bool {
            false $(|| socket_present!(event, $field, $role))*
        }

        pub(super) fn immediate_change(event: &TelemetryEvent, previous: &TelemetryEvent) -> bool {
            false $(|| immediate!(event, previous, $field, $change))*
        }

        pub(super) fn build_snapshot(event: &TelemetryEvent, motion: Option<&ClosureMotionMap>) -> String {
            let mut data = Map::new();
            $(project!(data, event, $field, $kind, $wire);)*
            if let Some((lat, lng)) = event.latitude.zip(event.longitude) {
                data.insert("location".to_owned(), json!({"lat":lat,"lng":lng}));
            }
            insert(&mut data, "software_update_status",
                event.ota_status.as_deref().or(event.ota_current_status.as_deref()));
            data.insert("is_online".to_owned(), json!(event.is_online.unwrap_or(true)));
            insert(&mut data, "closure_motion", motion);
            json!({"type":"status","ts":event.ts,"data":data}).to_string()
        }
    };
}

telemetry_fields!(define_values);
