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
