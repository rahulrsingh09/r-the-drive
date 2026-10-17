use super::*;
use serde_json::{json, Value};
use sqlx::Executor;
mod artwork;
mod enrollment;
mod extra;
mod support;
use support::*;

#[tokio::test]
#[ignore = "requires unique disposable restricted-role DB and Redis"]
async fn workflow_contract_management() {
    let f = Fixture::new().await;
    f.seed().await;
    let mut e = Evidence::default();
    let vehicle = format!("/v1/vehicles/{}", id(101));
    e.record(
        &f,
        "initial-list",
        f.request(Some(1), "GET", "/v1/vehicles", Value::Null).await,
    )
    .await;
    for (n, role) in [
        (1, "owner"),
        (2, "manager"),
        (3, "viewer"),
        (4, "foreign"),
        (5, "admin"),
    ] {
        for (method, suffix, body) in [
            ("GET", "/members", Value::Null),
            (
                "PUT",
                "/settings",
                json!({"battery_capacity_kwh":135.5,"battery_config":"Large","target_tire_pressure_psi":48.0}),
            ),
            ("PUT", "/name", json!({"name":"  Contract vehicle  "})),
            ("POST", "/default", Value::Null),
            ("GET", "/invites", Value::Null),
            ("PUT", "/credentials", json!({})),
            ("DELETE", "", Value::Null),
        ] {
            e.record(
                &f,
                &format!("{role}-{method}{suffix}"),
                f.request(Some(n), method, &format!("{vehicle}{suffix}"), body)
                    .await,
            )
            .await;
        }
    }
    for (i, body) in [
        json!({}),
        json!({"battery_capacity_kwh":null,"battery_config":null,"target_tire_pressure_psi":null}),
        json!({"battery_capacity_kwh":0,"battery_config":"Max","target_tire_pressure_psi":20}),
        json!({"battery_capacity_kwh":-1,"battery_config":"unknown","target_tire_pressure_psi":80}),
        json!({"target_tire_pressure_psi":19.999}),
        json!({"target_tire_pressure_psi":80.001}),
        json!({"battery_capacity_kwh":"bad"}),
    ]
    .into_iter()
    .enumerate()
    {
        e.record(
            &f,
            &format!("settings-{i}"),
            f.request(Some(1), "PUT", &format!("{vehicle}/settings"), body)
                .await,
        )
        .await;
    }
    for name in ["", "  ", "Demo R2S", "é"] {
        e.record(
            &f,
            &format!("name-{name}"),
            f.request(
                Some(1),
                "PUT",
                &format!("{vehicle}/name"),
                json!({"name":name}),
            )
            .await,
        )
        .await;
    }
    for (i, (path, body)) in [
        ("members", json!({"email":"", "role":"invalid"})),
        ("invites", json!({"email":"", "role":"invalid"})),
        (
            "members",
            json!({"email":"recipient@example.invalid", "role":"invalid"}),
        ),
        (
            "members",
            json!({"email":" RECIPIENT@example.invalid ", "role":"viewer"}),
        ),
        (
            "members",
            json!({"email":"recipient@example.invalid", "role":"manager"}),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        e.record(
            &f,
            &format!("membership-{i}"),
            f.request(Some(1), "POST", &format!("{vehicle}/{path}"), body)
                .await,
        )
        .await;
    }
    for (n, role) in [(6, "viewer"), (77, "viewer"), (6, "invalid")] {
        e.record(
            &f,
            &format!("update-member-{n}-{role}"),
            f.request(
                Some(1),
                "PUT",
                &format!("{vehicle}/members/{}", id(n)),
                json!({"role":role}),
            )
            .await,
        )
        .await;
    }
    for n in [1, 6, 6] {
        e.record(
            &f,
            &format!("remove-member-{n}"),
            f.request(
                Some(1),
                "DELETE",
                &format!("{vehicle}/members/{}", id(n)),
                Value::Null,
            )
            .await,
        )
        .await;
    }
    e.record(
        &f,
        "switch-default",
        f.request(
            Some(1),
            "POST",
            &format!("/v1/vehicles/{}/default", id(102)),
            Value::Null,
        )
        .await,
    )
    .await;
    e.record(
        &f,
        "repeat-default",
        f.request(
            Some(1),
            "POST",
            &format!("/v1/vehicles/{}/default", id(102)),
            Value::Null,
        )
        .await,
    )
    .await;
    e.record(
        &f,
        "foreign-default",
        f.request(
            Some(1),
            "POST",
            &format!("/v1/vehicles/{}/default", id(103)),
            Value::Null,
        )
        .await,
    )
    .await;
    for (method, path, bytes) in [
        ("POST", format!("{vehicle}/members"), b"{".to_vec()),
        ("PUT", format!("{vehicle}/settings"), b"null".to_vec()),
        (
            "PUT",
            format!("{vehicle}/name"),
            vec![b'x'; 2 * 1024 * 1024 + 1],
        ),
    ] {
        e.record(
            &f,
            "malformed-or-oversized",
            f.raw(Some(1), method, &path, bytes).await,
        )
        .await;
    }
    e.record(
        &f,
        "unauthenticated",
        f.request(None, "GET", "/v1/vehicles", Value::Null).await,
    )
    .await;
    let mut scoped = f.auth(1);
    scoped.api_access_level = Some("read".into());
    scoped.api_vehicle_id = Some(id(102));
    e.push(
        "scoped-list",
        result(list_vehicles(State(f.state.clone()), scoped.clone()).await).await,
    );
    e.push(
        "scoped-default-denied",
        result(set_default_vehicle(State(f.state.clone()), scoped, Path(id(102))).await).await,
    );
    let before = f.snapshot().await;
    f.state
        .pool
        .execute("REVOKE UPDATE ON panda.users FROM CURRENT_USER")
        .await
        .unwrap();
    let failed = f
        .request(Some(1), "POST", &format!("{vehicle}/default"), Value::Null)
        .await;
    assert_eq!(
        before,
        f.snapshot().await,
        "default late failure must roll back complete state"
    );
    e.push("default-late-failure", failed);
    f.state
        .pool
        .execute("GRANT UPDATE ON panda.users TO CURRENT_USER")
        .await
        .unwrap();
    e.record(
        &f,
        "default-retry",
        f.request(Some(1), "POST", &format!("{vehicle}/default"), Value::Null)
            .await,
    )
    .await;
    f.cleanup().await;
    e.finish("management", include_str!("management.json"));
}

#[tokio::test]
#[ignore = "requires unique disposable restricted-role DB and Redis"]
async fn workflow_contract_invitations() {
    let f = Fixture::new().await;
    f.seed().await;
    let mut e = Evidence::default();
    let path = format!("/v1/vehicles/{}/invites", id(101));
    for (i, days) in [(0, None), (1, Some(-5)), (2, Some(90))] {
        let before = Utc::now();
        let r=f.request(Some(1),"POST",&path,json!({"email":" Recipient@Example.Invalid ","role":"viewer","expires_in_days":days})).await;
        assert_eq!(r["status"], 200);
        let expiry = r["body"]["expires_at"].as_str().unwrap();
        let parsed: chrono::DateTime<Utc> = expiry.parse().unwrap();
        let delta = (parsed - before).num_seconds();
        let expected = days.unwrap_or(14).clamp(1, 30) as i64 * 86400;
        assert!((expected - 1..=expected + 1).contains(&delta));
        e.identity(
            expiry,
            &format!(
                "<generated-clock-plus-{}-days>",
                days.unwrap_or(14).clamp(1, 30)
            ),
        );
        let token = r["body"]["invite_token"].as_str().unwrap().to_owned();
        let invite_id: Uuid =
            sqlx::query_scalar("SELECT id FROM panda.vehicle_invites WHERE token_hash=$1")
                .bind(hash_invite_token(&token))
                .fetch_one(&f.state.pool)
                .await
                .unwrap();
        e.identity(invite_id, &format!("<invite-{i}>"));
        e.identity(&token, &format!("<random-invite-token-{i}>"));
        e.identity(
            format!("\\x{}", hex::encode(hash_invite_token(&token))),
            &format!("<hash-of-random-invite-token-{i}>"),
        );
        // PostgreSQL JSONB uses +00:00 while the response uses Z.
        e.identity(
            parsed.to_rfc3339(),
            &format!(
                "<generated-clock-plus-{}-days>",
                days.unwrap_or(14).clamp(1, 30)
            ),
        );
        let stored: Value = sqlx::query_scalar(
            "SELECT to_jsonb(expires_at) FROM panda.vehicle_invites WHERE id=$1",
        )
        .bind(invite_id)
        .fetch_one(&f.state.pool)
        .await
        .unwrap();
        e.identity(
            stored.as_str().unwrap(),
            &format!(
                "<generated-clock-plus-{}-days>",
                days.unwrap_or(14).clamp(1, 30)
            ),
        );
        let stored_time: chrono::DateTime<Utc> = stored.as_str().unwrap().parse().unwrap();
        e.identity(
            stored_time.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true),
            &format!(
                "<generated-clock-plus-{}-days>",
                days.unwrap_or(14).clamp(1, 30)
            ),
        );
        e.record(&f, &format!("create-{i}"), r).await;
        e.record(
            &f,
            &format!("preview-{i}"),
            f.request(None, "GET", &format!("/v1/invites/{token}"), Value::Null)
                .await,
        )
        .await;
        e.record(
            &f,
            &format!("wrong-user-{i}"),
            f.request(
                Some(4),
                "POST",
                &format!("/v1/invites/{token}/accept"),
                Value::Null,
            )
            .await,
        )
        .await;
        if i == 0 {
            let before = f.snapshot().await;
            f.state
                .pool
                .execute("REVOKE UPDATE ON panda.vehicle_invites FROM CURRENT_USER")
                .await
                .unwrap();
            let r = f
                .request(
                    Some(6),
                    "POST",
                    &format!("/v1/invites/{token}/accept"),
                    Value::Null,
                )
                .await;
            assert_eq!(
                before,
                f.snapshot().await,
                "acceptance late failure is atomic"
            );
            e.push("accept-late-failure", r);
            f.state
                .pool
                .execute("GRANT UPDATE ON panda.vehicle_invites TO CURRENT_USER")
                .await
                .unwrap();
            e.record(
                &f,
                "accept-retry",
                f.request(
                    Some(6),
                    "POST",
                    &format!("/v1/invites/{token}/accept"),
                    Value::Null,
                )
                .await,
            )
            .await;
            e.record(
                &f,
                "accept-replay",
                f.request(
                    Some(6),
                    "POST",
                    &format!("/v1/invites/{token}/accept"),
                    Value::Null,
                )
                .await,
            )
            .await;
        } else {
            e.record(
                &f,
                &format!("revoke-{i}"),
                f.request(
                    Some(1),
                    "DELETE",
                    &format!("{path}/{invite_id}"),
                    Value::Null,
                )
                .await,
            )
            .await;
            e.record(
                &f,
                &format!("accept-revoked-{i}"),
                f.request(
                    Some(6),
                    "POST",
                    &format!("/v1/invites/{token}/accept"),
                    Value::Null,
                )
                .await,
            )
            .await;
        }
    }
    for (n, revoked, accepted, expires) in [
        (201, true, true, "2020-01-01T00:00:00Z"),
        (202, false, true, "2020-01-01T00:00:00Z"),
        (203, false, false, "2020-01-01T00:00:00Z"),
        (204, false, false, "2099-01-01T00:00:00Z"),
    ] {
        let token = format!("fixed-invite-{n}");
        sqlx::query("INSERT INTO panda.vehicle_invites(id,vehicle_id,invited_by,invitee_email,role,token_hash,expires_at,revoked_at,accepted_at,created_at,updated_at) VALUES($1,$2,$3,'recipient@example.invalid','manager',$4,$5,CASE WHEN $6 THEN '2020-01-01Z'::timestamptz END,CASE WHEN $7 THEN '2020-01-01Z'::timestamptz END,'2026-01-01Z','2026-01-01Z')").bind(id(n)).bind(id(101)).bind(id(1)).bind(hash_invite_token(&token)).bind(expires.parse::<chrono::DateTime<Utc>>().unwrap()).bind(revoked).bind(accepted).execute(&f.state.pool).await.unwrap();
        e.record(
            &f,
            &format!("priority-{n}"),
            f.request(
                Some(4),
                "POST",
                &format!("/v1/invites/{token}/accept"),
                Value::Null,
            )
            .await,
        )
        .await;
    }
    let token = "fixed-invite-204";
    let accept_path = format!("/v1/invites/{token}/accept");
    let (a, b) = tokio::join!(
        f.request(Some(6), "POST", &accept_path, Value::Null),
        f.request(Some(6), "POST", &accept_path, Value::Null)
    );
    let mut pair = vec![a, b];
    pair.sort_by_key(Value::to_string);
    assert_eq!(pair.iter().filter(|r| r["status"] == 200).count(), 1);
    e.record(&f, "concurrent-accept", json!(pair)).await;
    e.record(
        &f,
        "list-invites",
        f.request(Some(1), "GET", &path, Value::Null).await,
    )
    .await;
    f.cleanup().await;
    e.finish("invitations", include_str!("invitations.json"));
}

#[tokio::test]
#[ignore = "requires unique disposable restricted-role DB and Redis"]
async fn workflow_contract_schedules() {
    let f = Fixture::new().await;
    f.seed().await;
    let mut e = Evidence::default();
    for suffix in [
        "charging-schedule",
        "departure-schedules",
        "wallboxes",
        "ota-details",
    ] {
        e.record(
            &f,
            &format!("empty-{suffix}"),
            f.request(
                Some(1),
                "GET",
                &format!("/v1/vehicles/{}/{suffix}", id(101)),
                Value::Null,
            )
            .await,
        )
        .await;
    }
    f.state.pool.execute(sqlx::AssertSqlSafe(format!("INSERT INTO panda.charging_schedules(id,vehicle_id,enabled,start_time_minutes,duration_minutes,amperage,location_lat,location_lng,week_days,rivian_updated_at,created_at,updated_at) VALUES('{}','{}',true,0,1440,32.5,0,-180,ARRAY['mon','sun'],'2026-01-01Z','2026-01-01Z','2026-01-01Z'); INSERT INTO panda.departure_schedules(id,vehicle_id,rivian_schedule_id,name,enabled,occurrence,comfort_settings,created_at,updated_at) VALUES('{}','{}','schedule-two',NULL,false,NULL,'{{}}','2026-01-02Z','2026-01-01Z'),('{}','{}','schedule-one','Morning',true,'{{\"days\":[\"mon\"]}}','{{\"temperature\":20}}','2026-01-01Z','2026-01-01Z'); INSERT INTO panda.wallboxes(id,user_id,rivian_wallbox_id,name,max_power_kw,linked,created_at,updated_at) VALUES('{}','{}','box-owner','Garage',11.5,true,'2026-01-01Z','2026-01-01Z'),('{}','{}','box-foreign',NULL,NULL,NULL,'2026-01-01Z','2026-01-01Z');",id(301),id(101),id(302),id(101),id(303),id(101),id(304),id(1),id(305),id(4)))).await.unwrap();
    sqlx::query("INSERT INTO timeseries.telemetry(vehicle_id,ts,ota_current_version,ota_available_version) VALUES($1,'2026-01-01Z','v1','v2'),($1,'2026-01-02Z',NULL,'v3'),($1,'2026-01-03Z',NULL,NULL)").bind(id(101)).execute(&f.state.pool).await.unwrap();
    for n in [1, 2, 3, 4, 5] {
        for suffix in [
            "charging-schedule",
            "departure-schedules",
            "wallboxes",
            "ota-details",
        ] {
            e.record(
                &f,
                &format!("read-{n}-{suffix}"),
                f.request(
                    Some(n),
                    "GET",
                    &format!("/v1/vehicles/{}/{suffix}", id(101)),
                    Value::Null,
                )
                .await,
            )
            .await;
        }
        for (method, suffix) in [
            ("PUT", "charging-schedule"),
            ("POST", "departure-schedules"),
            ("PATCH", "departure-schedules/malformed"),
            ("DELETE", "departure-schedules/malformed"),
        ] {
            e.record(
                &f,
                &format!("denied-{n}-{method}"),
                f.raw(
                    Some(n),
                    method,
                    &format!("/v1/vehicles/{}/{suffix}", id(101)),
                    vec![b'x'; 2 * 1024 * 1024 + 1],
                )
                .await,
            )
            .await;
        }
    }
    e.push(
        "unauthenticated-write",
        f.raw(
            None,
            "PUT",
            &format!("/v1/vehicles/{}/charging-schedule", id(101)),
            b"{".to_vec(),
        )
        .await,
    );
    f.cleanup().await;
    e.finish("schedules", include_str!("schedules.json"));
}

#[tokio::test]
#[ignore = "requires unique disposable restricted-role DB and Redis"]
async fn full_status_response_characterization() {
    let f = Fixture::new().await;
    f.seed().await;
    let mut responses = Vec::new();
    let path = format!("/v1/vehicles/{}/status", id(101));
    let empty = f.request(Some(1), "GET", &path, Value::Null).await;
    assert_eq!(empty["status"], 200);
    assert_eq!(empty["body"]["battery_level"], Value::Null);
    assert_eq!(empty["body"]["open_closures"], Value::Null);
    assert_eq!(empty["body"]["is_online"], false);
    assert_eq!(empty["body"]["telemetry_stale"], false);
    let expected_keys = &["alarm_active", "altitude_m", "auth_reason_code", "auth_state", "battery_capacity_kwh", "battery_level", "battery_level_ts", "battery_limit", "battery_limit_ts", "brake_fluid_low", "cabin_precon_status", "cabin_precon_type", "cabin_temp_c", "charge_port_open", "charger_derate_active", "charger_state", "charger_state_ts", "charger_status", "charger_status_ts", "closure_frunk_closed", "closure_frunk_locked", "closure_liftgate_closed", "closure_liftgate_locked", "closure_tailgate_closed", "closure_tailgate_locked", "credential_issued_at", "defrost_active", "door_front_left_closed", "door_front_left_locked", "door_front_right_closed", "door_front_right_locked", "door_rear_left_closed", "door_rear_left_locked", "door_rear_right_closed", "door_rear_right_locked", "doors_locked", "drive_mode", "driver_temp_c", "expected_renewal_at", "field_availability", "gear_guard_locked", "gear_guard_video_status", "gear_status", "heading_deg", "hv_thermal_event", "is_online", "last_charge_history_success_at", "last_charge_history_sync_at", "last_event_at", "last_heartbeat_at", "last_payload_at", "last_updated", "last_ws_heartbeat_received_at", "last_ws_payload_received_at", "last_ws_received_at", "latitude", "location_ts", "longitude", "odometer_miles", "odometer_miles_ts", "open_closures", "ota_available_version", "ota_current_status", "ota_current_version", "ota_status", "outside_temp_c", "pet_mode_active", "pet_mode_temp_ok", "power_state", "power_state_ts", "range_miles", "range_miles_ts", "renewal_state", "seat_fl_heat", "seat_fl_vent", "seat_fr_heat", "seat_fr_vent", "seat_rl_heat", "seat_rr_heat", "service_mode", "side_bin_left_closed", "side_bin_left_locked", "side_bin_right_closed", "side_bin_right_locked", "software_update_status", "speed_mph", "speed_mph_ts", "steering_wheel_heat", "telemetry_stale", "telemetry_stale_reason", "time_to_end_of_charge_min", "time_to_end_of_charge_min_ts", "tire_fl_psi", "tire_fl_status", "tire_fl_valid", "tire_fr_psi", "tire_fr_status", "tire_fr_valid", "tire_min_psi", "tire_pressure_status", "tire_rl_psi", "tire_rl_status", "tire_rl_valid", "tire_rr_psi", "tire_rr_status", "tire_rr_valid", "tonneau_closed", "tonneau_locked", "twelve_volt_health", "vehicle_id", "window_fl_closed", "window_fr_closed", "window_rl_closed", "window_rr_closed", "wiper_fluid_low", "worker_health", "worker_health_msg"];
    let actual_keys: Vec<_> = empty["body"].as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(actual_keys, expected_keys);
    responses.push(empty);
    let mut telemetry: Value = serde_json::from_str(r#"{"ts": "2026-01-03T04:05:06Z", "latitude": 1.25, "longitude": 2.25, "altitude_m": 3.25, "speed_mph": 4.25, "location_ts": "2026-01-03T04:05:06Z", "speed_mph_ts": "2026-01-03T04:05:06Z", "battery_level": 75.0, "battery_capacity_wh": 135000.0, "distance_to_empty_mi": 200.0, "battery_limit": 10.25, "battery_level_ts": "2026-01-03T04:05:06Z", "distance_to_empty_mi_ts": "2026-01-03T04:05:06Z", "battery_limit_ts": "2026-01-03T04:05:06Z", "power_state": "fixture-power_state", "power_state_ts": "2026-01-03T04:05:06Z", "charger_state": "fixture-charger_state", "charger_state_ts": "2026-01-03T04:05:06Z", "charger_status": "fixture-charger_status", "charger_status_ts": "2026-01-03T04:05:06Z", "time_to_end_of_charge_min": 20, "time_to_end_of_charge_min_ts": "2026-01-03T04:05:06Z", "drive_mode": "fixture-drive_mode", "gear_status": "fixture-gear_status", "cabin_temp_c": 24.25, "driver_temp_c": 25.25, "outside_temp_c": 26.25, "heading_deg": 27.25, "odometer_miles": 28.25, "odometer_miles_ts": "2026-01-03T04:05:06Z", "tire_fl_psi": 30.25, "tire_fr_psi": 31.25, "tire_rl_psi": 32.25, "tire_rr_psi": 33.25, "tire_fl_status": "ok", "tire_fr_status": "low", "tire_rl_status": "unknown", "tire_rr_status": "ok", "tire_fl_valid": true, "tire_fr_valid": true, "tire_rl_valid": true, "tire_rr_valid": true, "door_front_left_locked": true, "door_front_right_locked": true, "door_rear_left_locked": true, "door_rear_right_locked": true, "door_front_left_closed": true, "door_front_right_closed": false, "door_rear_left_closed": true, "door_rear_right_closed": false, "closure_frunk_locked": true, "closure_frunk_closed": false, "closure_liftgate_locked": true, "closure_liftgate_closed": false, "closure_tailgate_locked": true, "closure_tailgate_closed": false, "ota_current_version": "fixture-ota_current_version", "ota_available_version": "fixture-ota_available_version", "ota_status": "fixture-ota_status", "ota_current_status": "fixture-ota_current_status", "hv_thermal_event": "fixture-hv_thermal_event", "twelve_volt_health": "fixture-twelve_volt_health", "charge_port_open": true, "charger_derate_active": false, "cabin_precon_status": "fixture-cabin_precon_status", "cabin_precon_type": "fixture-cabin_precon_type", "pet_mode_active": true, "pet_mode_temp_ok": false, "defrost_active": true, "steering_wheel_heat": 69, "seat_fl_heat": 70, "seat_fr_heat": 71, "seat_rl_heat": 72, "seat_rr_heat": 73, "seat_fl_vent": 74, "seat_fr_vent": 75, "tonneau_locked": true, "tonneau_closed": false, "side_bin_left_locked": true, "side_bin_right_locked": false, "side_bin_left_closed": true, "side_bin_right_closed": false, "window_fl_closed": true, "window_fr_closed": false, "window_rl_closed": true, "window_rr_closed": false, "gear_guard_locked": true, "gear_guard_video_status": "fixture-gear_guard_video_status", "wiper_fluid_low": true, "brake_fluid_low": false, "alarm_active": true, "service_mode": false}"#).unwrap();
    telemetry["vehicle_id"] = json!(id(101));
    telemetry["updated_at"] = json!("2026-01-03T04:05:06Z");
    sqlx::query("INSERT INTO panda.vehicle_latest_status SELECT (jsonb_populate_record(NULL::panda.vehicle_latest_status,$1)).*")
        .bind(&telemetry).execute(&f.state.pool).await.unwrap();
    sqlx::query("INSERT INTO timeseries.telemetry(vehicle_id,ts,tire_fl_psi,tire_fr_status,tire_rl_valid,door_front_left_closed,hv_thermal_event) VALUES($1,'2026-01-02Z',42,'low',true,false,'none')")
        .bind(id(101)).execute(&f.state.pool).await.unwrap();
    sqlx::query("INSERT INTO panda.vehicle_runtime_state(vehicle_id,is_online,last_event_at,last_payload_at,last_heartbeat_at,last_ws_received_at,last_ws_payload_received_at,last_ws_heartbeat_received_at,last_charge_history_sync_at,last_charge_history_success_at,worker_health,worker_health_msg,auth_state,auth_reason_code) VALUES($1,true,'2026-01-04Z','2026-01-05Z','2026-01-06Z','2026-01-07Z','2026-01-08Z','2026-01-09Z','2026-01-10Z','2026-01-11Z','connected','fixture health','authorized','fixture reason')")
        .bind(id(101)).execute(&f.state.pool).await.unwrap();
    let dense = f.request(Some(1), "GET", &path, Value::Null).await;
    assert_eq!(dense["status"], 200);
    assert_eq!(dense["body"]["battery_capacity_kwh"], 135.0);
    assert_eq!(dense["body"]["doors_locked"], true);
    assert_eq!(dense["body"]["tire_pressure_status"], "low");
    assert_eq!(dense["body"]["last_updated"], "2026-01-03T04:05:06Z");
    assert_eq!(dense["body"]["last_event_at"], "2026-01-04T00:00:00Z");
    assert_eq!(dense["body"]["telemetry_stale_reason"], "ws_silent");
    assert_eq!(dense["body"]["field_availability"]["tire_fl_psi"]["availability"], "historical");
    assert_eq!(dense["body"]["field_availability"]["tire_fr_psi"]["availability"], "never_seen");
    responses.push(dense);
    sqlx::query("UPDATE panda.vehicle_latest_status SET tire_fl_valid=false,door_front_left_locked=NULL,battery_capacity_wh=1000,battery_level_ts=NULL,distance_to_empty_mi_ts=NULL,power_state_ts=NULL WHERE vehicle_id=$1")
        .bind(id(101)).execute(&f.state.pool).await.unwrap();
    sqlx::query("UPDATE panda.vehicle_runtime_state SET last_ws_received_at=NULL WHERE vehicle_id=$1")
        .bind(id(101)).execute(&f.state.pool).await.unwrap();
    let sparse = f.request(Some(1), "GET", &path, Value::Null).await;
    assert_eq!(sparse["body"]["battery_capacity_kwh"], 1000.0);
    assert_eq!(sparse["body"]["doors_locked"], Value::Null);
    assert_eq!(sparse["body"]["battery_level_ts"], "2026-01-03T04:05:06Z");
    assert_eq!(sparse["body"]["tire_pressure_status"], "invalid_sensor");
    assert_eq!(sparse["body"]["telemetry_stale_reason"], "battery_stale");
    assert_eq!(sparse["body"]["field_availability"]["tire_fl_psi"]["reason_code"], "invalid_sensor");
    responses.push(sparse);
    for user in [None, Some(2), Some(3), Some(4), Some(5)] {
        let response = f.request(user, "GET", &path, Value::Null).await;
        assert_eq!(response["status"], if user.is_none() {401} else if matches!(user,Some(2)|Some(3)) {200} else {403});
        responses.push(response);
    }
    // Query failures must fail the response instead of silently dropping status fields.
    sqlx::query("ALTER TABLE panda.vehicle_latest_status RENAME TO vehicle_latest_status_fault")
        .execute(&f.state.pool).await.unwrap();
    let failed = f.request(Some(1), "GET", &path, Value::Null).await;
    assert_eq!(failed["status"], 500);
    responses.push(failed);
    sqlx::query("ALTER TABLE panda.vehicle_latest_status_fault RENAME TO vehicle_latest_status")
        .execute(&f.state.pool).await.unwrap();
    assert_eq!(f.request(Some(1), "GET", &path, Value::Null).await["status"], 200);
    let evidence = json!(responses);
    if let Ok(path) = std::env::var("STATUS_CHARACTERIZATION_CAPTURE") {
        std::fs::write(path, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
    }
    if let Ok(path) = std::env::var("STATUS_CHARACTERIZATION_COMPARE") {
        let original: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(evidence, original, "all status response fields, headers, and failure bodies");
    }
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires unique disposable restricted-role DB and Redis"]
async fn capture_dispatch_and_paged_export_boundaries() {
    let f = Fixture::new().await;
    f.seed().await;
    let vehicle = id(101);
    assert!(matches!(
        export_ingestion_capture(State(f.state.clone()), f.auth(1), Path(vehicle)).await,
        Err(AppError::NotFound)
    ));
    for user in [3, 4, 5] {
        assert!(matches!(
            get_ingestion_diagnostics(State(f.state.clone()), f.auth(user), Path(vehicle)).await,
            Err(AppError::Forbidden)
        ));
        assert!(matches!(
            start_ingestion_capture(State(f.state.clone()), f.auth(user), Path(vehicle)).await,
            Err(AppError::Forbidden)
        ));
    }
    let mut api = f.auth(1);
    api.api_access_level = Some("read".into());
    api.api_vehicle_id = Some(vehicle);
    assert!(matches!(
        get_ingestion_diagnostics(State(f.state.clone()), api, Path(vehicle)).await,
        Err(AppError::Forbidden)
    ));
    assert!(
        start_ingestion_capture(State(f.state.clone()), f.auth(1), Path(vehicle))
            .await
            .is_err()
    );
    sqlx::query(
        "UPDATE panda.vehicles SET rivian_vehicle_id='synthetic-capture-remote' WHERE id=$1",
    )
    .bind(vehicle)
    .execute(&f.state.pool)
    .await
    .unwrap();
    let Json(started) = start_ingestion_capture(State(f.state.clone()), f.auth(1), Path(vehicle))
        .await
        .unwrap();
    assert_eq!(serde_json::to_value(started).unwrap()["state"], "capturing");
    let capture: Uuid = sqlx::query_scalar(
        "SELECT capture_id FROM panda.vehicle_ingestion_diagnostics WHERE vehicle_id=$1",
    )
    .bind(vehicle)
    .fetch_one(&f.state.pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO panda.vehicle_ingestion_capture_events(vehicle_id,capture_id,recorded_at,kind,fields) SELECT $1,$2,'2026-01-01Z','fixture',jsonb_build_object('sequence',n) FROM generate_series(1,1005) n").bind(vehicle).bind(capture).execute(&f.state.pool).await.unwrap();
    let exported = export_ingestion_capture(State(f.state.clone()), f.auth(2), Path(vehicle))
        .await
        .unwrap();
    assert_eq!(
        exported.headers()[header::CONTENT_TYPE],
        "application/x-ndjson"
    );
    assert_eq!(exported.headers()[header::CACHE_CONTROL], "no-store");
    assert!(exported.headers()[header::CONTENT_DISPOSITION]
        .to_str()
        .unwrap()
        .starts_with("attachment; filename="));
    let bytes = axum::body::to_bytes(exported.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    let lines: Vec<Value> = std::str::from_utf8(&bytes)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let sequence: Vec<_> = lines
        .iter()
        .filter_map(|line| line["fields"]["sequence"].as_i64())
        .collect();
    assert_eq!(sequence, (1..=1005).collect::<Vec<_>>());
    assert!(!String::from_utf8(bytes.to_vec())
        .unwrap()
        .contains(&vehicle.to_string()));
    let Json(stopped) = stop_ingestion_capture(State(f.state.clone()), f.auth(2), Path(vehicle))
        .await
        .unwrap();
    assert_eq!(serde_json::to_value(stopped).unwrap()["state"], "stopped");
    let failed_stream = export_ingestion_capture(State(f.state.clone()), f.auth(1), Path(vehicle))
        .await
        .unwrap();
    sqlx::query(
        "ALTER TABLE panda.vehicle_ingestion_capture_events RENAME TO capture_events_fault",
    )
    .execute(&f.state.pool)
    .await
    .unwrap();
    assert!(
        axum::body::to_bytes(failed_stream.into_body(), 4 * 1024 * 1024)
            .await
            .is_err()
    );
    sqlx::query(
        "ALTER TABLE panda.capture_events_fault RENAME TO vehicle_ingestion_capture_events",
    )
    .execute(&f.state.pool)
    .await
    .unwrap();
    f.cleanup().await;
}
