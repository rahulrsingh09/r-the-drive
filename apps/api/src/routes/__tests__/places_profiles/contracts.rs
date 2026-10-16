use super::*;
use axum::{
    body::{to_bytes, Body},
    extract::ConnectInfo,
    http::Request,
};
use serde_json::json;
use sqlx::Executor;
use std::{collections::BTreeMap, net::SocketAddr, sync::Arc};
use tower::ServiceExt;
#[path = "database.rs"]
mod database;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

#[derive(Default)]
struct Evidence {
    ids: BTreeMap<String, String>,
    events: Vec<Value>,
    raw: Vec<Value>,
}
impl Evidence {
    fn normalize(&mut self, value: &mut Value) {
        match value {
            Value::String(s) => {
                if let Ok(uuid) = Uuid::parse_str(s) {
                    if uuid.as_u128() > 1000 {
                        let next = format!("generated-{}", self.ids.len());
                        *s = self.ids.entry(s.clone()).or_insert(next).clone();
                    }
                } else if chrono::DateTime::parse_from_rfc3339(s).is_ok()
                    && !s.starts_with("2026-01-")
                {
                    *s = "<server-time>".into();
                }
            }
            Value::Array(a) => {
                for v in a {
                    self.normalize(v);
                }
            }
            Value::Object(o) => {
                for (k, v) in o {
                    if k == "usage_date" && !v.is_null() {
                        *v = json!("<server-date>");
                    } else {
                        self.normalize(v);
                    }
                }
            }
            _ => {}
        }
    }
    async fn record(&mut self, state: &AppState, label: &str, response: Value) {
        let mut response = response;
        let raw_response = response.clone();
        let mut raw_tables = BTreeMap::new();
        self.normalize(&mut response);
        let mut tables = BTreeMap::new();
        for table in [
            "geofences",
            "addresses",
            "cost_profiles",
            "charge_sessions",
            "vehicles",
            "vehicle_memberships",
            "external_connection_activity",
            "charge_session_user_annotations",
            "vehicle_user_settings",
        ] {
            let mut rows: Vec<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "SELECT to_jsonb(t) FROM panda.{table} t ORDER BY to_jsonb(t)::text"
            )))
            .fetch_all(&state.pool)
            .await
            .unwrap();
            raw_tables.insert(table, rows.clone());
            for row in &mut rows {
                self.normalize(row);
            }
            rows.sort_by_key(Value::to_string);
            tables.insert(table, rows);
        }
        self.raw
            .push(json!({"case":label,"response":raw_response,"persisted":raw_tables}));
        self.events
            .push(json!({"case":label,"response":response,"persisted":tables}));
    }
}

async fn request(state: &AppState, user: u128, method: &str, path: &str, body: String) -> Value {
    let token = if user == 0 {
        "invalid-synthetic-token".into()
    } else if user == 99 {
        "panda_synthetic_places_read_key".into()
    } else {
        crate::middleware::auth::issue_access_token(id(user), None, &state.jwt_keys).unwrap()
    };
    // A fresh production router isolates rate-limit clocks between synthetic cases.
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .header("x-request-id", "synthetic-places-request")
        .body(Body::from(body))
        .unwrap();
    request.extensions_mut().insert(ConnectInfo(
        "127.0.0.1:19321".parse::<SocketAddr>().unwrap(),
    ));
    let response = crate::routes::build_router(state.clone())
        .oneshot(request)
        .await
        .unwrap();
    let status = response.status().as_u16();
    let mut headers = BTreeMap::new();
    for (key, value) in response.headers() {
        headers.insert(key.as_str().to_owned(), value.to_str().unwrap().to_owned());
    }
    let raw = to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .unwrap();
    let body = serde_json::from_slice::<Value>(&raw)
        .unwrap_or_else(|_| json!(String::from_utf8_lossy(&raw)));
    json!({"status":status,"headers":headers,"body":body})
}

fn place(name: &str) -> Value {
    json!({"name":name,"address":{"display_name":"Synthetic street","latitude":47.5,"longitude":-122.3,"osm_id":42,"road":"Fixture road","city":"Synthetic city","raw":{"fixture":true}},"charging":{"billing_type":"flat","rate":0.25}})
}
async fn call(
    e: &mut Evidence,
    s: &AppState,
    label: &str,
    user: u128,
    method: &str,
    path: &str,
    body: Value,
) -> Value {
    let r = request(s, user, method, path, body.to_string()).await;
    e.record(s, label, r.clone()).await;
    r["body"].clone()
}

#[tokio::test]
#[ignore = "requires unique disposable PLACES_TEST_DATABASE_URL; authentic migrations and restricted app role"]
async fn complete_http_and_persistence_contract() {
    let db = database::DatabaseFixture::new().await;
    let pool = &db.pool;
    for (n, role, disabled) in [
        (1, "admin", false),
        (2, "user", false),
        (3, "user", false),
        (4, "user", true),
    ] {
        sqlx::query("INSERT INTO panda.users(id,email,role,is_disabled,created_at,updated_at) VALUES ($1,$2,$3,$4,'2026-01-01Z','2026-01-01Z')")
            .bind(id(n)).bind(format!("places-{n}@example.invalid")).bind(role).bind(disabled).execute(pool).await.unwrap();
    }
    for (n, owner) in [(10, 1), (11, 2)] {
        sqlx::query("INSERT INTO panda.vehicles(id,user_id,rivian_vehicle_id,model,created_at,updated_at) VALUES ($1,$2,$3,'synthetic','2026-01-01Z','2026-01-01Z')")
            .bind(id(n)).bind(id(owner)).bind(format!("synthetic-places-{n}")).execute(pool).await.unwrap();
    }
    for (n, user, role) in [(20, 1, "owner"), (21, 2, "viewer"), (22, 3, "manager")] {
        sqlx::query("INSERT INTO panda.vehicle_memberships(id,user_id,vehicle_id,role,created_at,updated_at) VALUES ($1,$2,$3,$4,'2026-01-01Z','2026-01-01Z')").bind(id(n)).bind(id(user)).bind(id(10)).bind(role).execute(pool).await.unwrap();
    }
    sqlx::query("INSERT INTO panda.api_keys(user_id,vehicle_id,key_hash,access_level,name) VALUES ($1,$2,$3,'read','Synthetic fixture')").bind(id(1)).bind(id(10)).bind(crate::routes::api_keys::hash_api_key("panda_synthetic_places_read_key")).execute(pool).await.unwrap();
    crate::services::external_connections::ensure_defaults(pool)
        .await
        .unwrap();
    let keys = crate::keys::generate_keys().unwrap();
    let state = AppState {
        pool: pool.clone(), redis: redis::Client::open("redis://redis:6379/").unwrap(),
        jwt_keys: Arc::new(crate::middleware::auth::JwtKeys::new(&keys.jwt_private_pem,&keys.jwt_public_pem).unwrap()),
        age_key:keys.age_key, config:serde_json::from_value(json!({"database_url":"postgres://synthetic-local-only","redis_url":"redis://127.0.0.1:1/","panda_env":"production"})).unwrap(),
        resources:Default::default(),nominatim_cache:Default::default(),supervisor:crate::ingestion::supervisor::SupervisorHandle::noop()
    };
    let mut e = Evidence::default();
    for path in ["/v1/places", "/v1/cost-profiles"] {
        for user in [0, 1, 4, 99] {
            call(
                &mut e,
                &state,
                &format!("empty-{path}-{user}"),
                user,
                "GET",
                path,
                Value::Null,
            )
            .await;
        }
    }
    let created = call(
        &mut e,
        &state,
        "create-place",
        1,
        "POST",
        "/v1/places",
        place(" Home "),
    )
    .await;
    let place_id = created["id"].as_str().expect("created place").to_string();
    let profile_id = created["charging"]["id"].as_str().unwrap().to_string();
    let place_path = format!("/v1/places/{place_id}");
    let profile_path = format!("/v1/cost-profiles/{profile_id}");
    for user in [1, 2, 3] {
        for (method, body) in [("GET", Value::Null), ("PUT", place(" Foreign edit "))] {
            call(
                &mut e,
                &state,
                &format!("place-{method}-{user}"),
                user,
                method,
                &place_path,
                body,
            )
            .await;
        }
        call(
            &mut e,
            &state,
            &format!("profile-get-{user}"),
            user,
            "GET",
            &profile_path,
            Value::Null,
        )
        .await;
    }
    for user in [2, 3] {
        call(
            &mut e,
            &state,
            &format!("foreign-delete-place-{user}"),
            user,
            "DELETE",
            &place_path,
            Value::Null,
        )
        .await;
        call(
            &mut e,
            &state,
            &format!("foreign-update-profile-{user}"),
            user,
            "PUT",
            &profile_path,
            json!({"rate":0.9}),
        )
        .await;
        call(
            &mut e,
            &state,
            &format!("foreign-delete-profile-{user}"),
            user,
            "DELETE",
            &profile_path,
            Value::Null,
        )
        .await;
    }
    for (n, vehicle, mode, cost) in [
        (100, 10, "automatic", 1.0),
        (101, 10, "manual", 17.0),
        (102, 10, "free", 0.0),
        (103, 11, "automatic", 1.0),
    ] {
        sqlx::query("INSERT INTO panda.charge_sessions(id,vehicle_id,started_at,ended_at,kwh_added,cost_override_mode,cost_override_usd,cost_usd,geofence_id,cost_profile_id,created_at) VALUES ($1,$2,'2026-01-01T01:00:00Z'::timestamptz + $7::integer * interval '1 second','2026-01-01T02:00:00Z',10,$3,CASE WHEN $3='manual' THEN $4 ELSE NULL END,$4,$5,$6,'2026-01-01Z')")
            .bind(id(n)).bind(id(vehicle)).bind(mode).bind(cost).bind(Uuid::parse_str(&place_id).unwrap()).bind(Uuid::parse_str(&profile_id).unwrap()).bind(n as i32).execute(pool).await.unwrap();
    }
    let mut updated = place("Updated");
    updated["charging"]["rate"] = json!(0.4);
    call(
        &mut e,
        &state,
        "linked-automatic-update",
        1,
        "PUT",
        &place_path,
        updated.clone(),
    )
    .await;
    call(
        &mut e,
        &state,
        "linked-repeat",
        1,
        "PUT",
        &place_path,
        updated.clone(),
    )
    .await;
    updated["charging"] = Value::Null;
    call(
        &mut e,
        &state,
        "unlink-profile",
        1,
        "PUT",
        &place_path,
        updated,
    )
    .await;
    call(
        &mut e,
        &state,
        "new-profile-after-unlink",
        1,
        "PUT",
        &place_path,
        place("Repriced"),
    )
    .await;
    // Force a later recomputation failure without relaxing any history privilege.
    pool.execute("CREATE FUNCTION panda.synthetic_fail_cost() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'synthetic recompute failure'; END $$; CREATE TRIGGER synthetic_fail_cost BEFORE UPDATE OF cost_usd ON panda.charge_sessions FOR EACH ROW EXECUTE FUNCTION panda.synthetic_fail_cost();").await.unwrap();
    let mut failing = place("Committed before cost failure");
    failing["charging"]["rate"] = json!(0.95);
    let result = request(&state, 1, "PUT", &place_path, failing.to_string()).await;
    assert_eq!(result["status"], 500);
    e.record(&state, "post-commit-recompute-failure", result)
        .await;
    let committed: String = sqlx::query_scalar("SELECT name FROM panda.geofences WHERE id=$1")
        .bind(Uuid::parse_str(&place_id).unwrap())
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(committed, "Committed before cost failure");
    pool.execute("DROP TRIGGER synthetic_fail_cost ON panda.charge_sessions; DROP FUNCTION panda.synthetic_fail_cost()").await.unwrap();
    pool.execute("CREATE FUNCTION panda.synthetic_fail_place() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'synthetic place failure'; END $$; CREATE TRIGGER synthetic_fail_place BEFORE UPDATE ON panda.geofences FOR EACH ROW EXECUTE FUNCTION panda.synthetic_fail_place();").await.unwrap();
    let before: Value =
        sqlx::query_scalar("SELECT jsonb_agg(to_jsonb(a) ORDER BY id) FROM panda.addresses a")
            .fetch_one(pool)
            .await
            .unwrap();
    let mut changed = place("Rollback");
    changed["address"]["display_name"] = json!("Must roll back");
    call(
        &mut e,
        &state,
        "transaction-rolls-back-address-profile",
        1,
        "PUT",
        &place_path,
        changed,
    )
    .await;
    let after: Value =
        sqlx::query_scalar("SELECT jsonb_agg(to_jsonb(a) ORDER BY id) FROM panda.addresses a")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(before, after);
    pool.execute("DROP TRIGGER synthetic_fail_place ON panda.geofences; DROP FUNCTION panda.synthetic_fail_place()").await.unwrap();
    call(
        &mut e,
        &state,
        "viewer-own-place-same-osm",
        2,
        "POST",
        "/v1/places",
        place("Viewer place"),
    )
    .await;
    call(
        &mut e,
        &state,
        "owner-list",
        1,
        "GET",
        "/v1/places",
        Value::Null,
    )
    .await;
    call(
        &mut e,
        &state,
        "viewer-list",
        2,
        "GET",
        "/v1/places",
        Value::Null,
    )
    .await;
    let mut no_osm = place("No OSM");
    no_osm["address"]["osm_id"] = Value::Null;
    no_osm["charging"] = Value::Null;
    let without_osm = call(
        &mut e,
        &state,
        "no-osm-1",
        1,
        "POST",
        "/v1/places",
        no_osm.clone(),
    )
    .await;
    call(
        &mut e,
        &state,
        "no-osm-repeat",
        1,
        "POST",
        "/v1/places",
        no_osm,
    )
    .await;
    let without_osm_path = format!("/v1/places/{}", without_osm["id"].as_str().unwrap());
    call(
        &mut e,
        &state,
        "delete-unlinked-place",
        1,
        "DELETE",
        &without_osm_path,
        Value::Null,
    )
    .await;
    call(
        &mut e,
        &state,
        "delete-unlinked-place-repeat",
        1,
        "DELETE",
        &without_osm_path,
        Value::Null,
    )
    .await;
    for (i, patch) in [
        json!({"name":""}),
        json!({"address":{"display_name":"","latitude":47,"longitude":-122}}),
        json!({"radius_m":0}),
        json!({"radius_m":-1}),
        json!({"charging":{"billing_type":"free","rate":0}}),
        json!({"charging":{"billing_type":"per_kwh","rate":-1}}),
        json!({"charging":{"billing_type":"tou","rate":0}}),
        json!({"charging":{"billing_type":"tou","rate":0,"timezone":"UTC","tou_periods":{}}}),
    ]
    .into_iter()
    .enumerate()
    {
        let mut body = place("Invalid");
        for (k, v) in patch.as_object().unwrap() {
            body[k] = v.clone();
        }
        call(
            &mut e,
            &state,
            &format!("invalid-place-{i}"),
            1,
            "POST",
            "/v1/places",
            body.clone(),
        )
        .await;
        call(
            &mut e,
            &state,
            &format!("invalid-foreign-place-{i}"),
            2,
            "PUT",
            &place_path,
            body,
        )
        .await;
    }
    for (i,body) in [json!({}),json!({"name":"","billing_type":"free","rate":0}),json!({"name":"Fixed","billing_type":"flat","rate":12,"session_fee":-3,"currency":"XYZ","timezone":"Not/AZone","effective_from":"2026-01-01"}),json!({"name":"bad","billing_type":"invalid","rate":-1}),json!({"name":"bad","billing_type":"per_kwh","rate":5.0001}),json!({"name":"edge","billing_type":"per_kwh","rate":5}),json!({"name":"tou","billing_type":"tou","rate":0,"timezone":"UTC","tou_periods":[{"label":"All","start_minute":0,"end_minute":1440,"rate":0.2}]})].into_iter().enumerate() {
        let result=call(&mut e,&state,&format!("profile-create-{i}"),1,"POST","/v1/cost-profiles",body).await;
        if let Some(new_id)=result["id"].as_str() {
            let path=format!("/v1/cost-profiles/{new_id}");
            call(&mut e,&state,&format!("profile-null-patch-{i}"),1,"PUT",&path,json!({"timezone":null,"effective_from":null,"name":null,"rate":null,"tou_periods":null})).await;
            call(&mut e,&state,&format!("profile-empty-patch-{i}"),1,"PUT",&path,json!({})).await;
            call(&mut e,&state,&format!("profile-delete-{i}"),1,"DELETE",&path,Value::Null).await;
            call(&mut e,&state,&format!("profile-delete-repeat-{i}"),1,"DELETE",&path,Value::Null).await;
        }
    }
    for path in ["/v1/places", "/v1/cost-profiles"] {
        for (label, raw) in [
            ("malformed", "{".to_owned()),
            ("missing", "{}".to_owned()),
            ("oversized", "x".repeat(65537)),
        ] {
            let r = request(&state, 1, "POST", path, raw).await;
            e.record(&state, &format!("{label}-{path}"), r).await;
        }
        call(
            &mut e,
            &state,
            &format!("bad-id-{path}"),
            1,
            "GET",
            &format!("{path}/invalid"),
            Value::Null,
        )
        .await;
        call(
            &mut e,
            &state,
            &format!("missing-id-{path}"),
            1,
            "GET",
            &format!("{path}/{}", id(999)),
            Value::Null,
        )
        .await;
    }
    call(
        &mut e,
        &state,
        "delete-linked-profile",
        1,
        "DELETE",
        &profile_path,
        Value::Null,
    )
    .await;
    call(
        &mut e,
        &state,
        "delete-linked-place",
        1,
        "DELETE",
        &place_path,
        Value::Null,
    )
    .await;
    call(
        &mut e,
        &state,
        "delete-place-repeat",
        1,
        "DELETE",
        &place_path,
        Value::Null,
    )
    .await;
    for query in [
        "",
        "ab",
        "ABC",
        "abc&limit=0",
        "abc&limit=255",
        "abc&limit=256",
        "abc&limit=-1",
    ] {
        call(
            &mut e,
            &state,
            &format!("search-{query}"),
            1,
            "GET",
            &format!("/v1/places/search?q={query}"),
            Value::Null,
        )
        .await;
    }
    let cached = json!([{"lat":"47.1","lon":"-122.1","display_name":"Local fixture","osm_id":44,"address":{"pedestrian":"Walk","town":"Town"}}, {"lat":47,"lon":"-122","display_name":"Bad numeric lat"}, {"lat":"1","lon":"2","display_name":"Fallback","address":{"road":false,"pedestrian":"Ignored","city":null,"town":"Ignored"}}]);
    state
        .nominatim_cache
        .write()
        .await
        .insert("abc".into(), (std::time::Instant::now(), cached));
    for query in ["ABC", "%20AbC%20&limit=1", "abc&limit=0"] {
        call(
            &mut e,
            &state,
            &format!("cached-{query}"),
            2,
            "GET",
            &format!("/v1/places/search?q={query}"),
            Value::Null,
        )
        .await;
    }
    state
        .nominatim_cache
        .write()
        .await
        .get_mut("abc")
        .unwrap()
        .0 = std::time::Instant::now() - Duration::from_secs(3601);
    call(
        &mut e,
        &state,
        "expired-cache-denied-provider",
        1,
        "GET",
        "/v1/places/search?q=abc",
        Value::Null,
    )
    .await;
    let mut redis = state
        .redis
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    state.nominatim_cache.write().await.insert(
        "persisted".into(),
        (
            std::time::Instant::now(),
            json!([{"lat":"3","lon":"4","display_name":"Memory fixture"}]),
        ),
    );
    for (label, payload) in [
        (
            "persistent-precedes-memory",
            json!([{"lat":"1","lon":"2","display_name":"Redis fixture"}]),
        ),
        ("persistent-nonarray", json!({"not":"an array"})),
    ] {
        let _: () = redis
            .set(search_cache_key("persisted", 5), payload.to_string())
            .await
            .unwrap();
        call(
            &mut e,
            &state,
            label,
            1,
            "GET",
            "/v1/places/search?q=persisted",
            Value::Null,
        )
        .await;
    }
    let _: () = redis
        .set(search_cache_key("persisted", 5), "malformed-json")
        .await
        .unwrap();
    call(
        &mut e,
        &state,
        "persistent-invalid-falls-through",
        1,
        "GET",
        "/v1/places/search?q=persisted",
        Value::Null,
    )
    .await;
    call(
        &mut e,
        &state,
        "persistent-key-limit-specific",
        1,
        "GET",
        "/v1/places/search?q=persisted&limit=1",
        Value::Null,
    )
    .await;
    let _: usize = redis.del(search_cache_key("persisted", 5)).await.unwrap();
    let error = pool
        .execute("DELETE FROM panda.vehicles WHERE id='00000000-0000-0000-0000-00000000000a'")
        .await
        .unwrap_err();
    assert_eq!(
        error.as_database_error().unwrap().code().as_deref(),
        Some("42501")
    );
    e.record(&state, "protected-history", json!({"sqlstate":"42501"}))
        .await;
    for (label, coordinate) in [
        (
            "missing-latitude",
            json!({"display_name":"Synthetic selection","longitude":-122}),
        ),
        (
            "missing-longitude",
            json!({"display_name":"Synthetic selection","latitude":47}),
        ),
        (
            "null-latitude",
            json!({"display_name":"Synthetic selection","latitude":null,"longitude":-122}),
        ),
        (
            "string-latitude",
            json!({"display_name":"Synthetic selection","latitude":"bad","longitude":-122}),
        ),
        (
            "out-of-range-finite",
            json!({"display_name":"Synthetic selection","latitude":91,"longitude":181}),
        ),
    ] {
        let body = json!({"name":"Coordinate boundary", "address":coordinate});
        call(&mut e, &state, label, 1, "POST", "/v1/places", body).await;
    }
    let raw_coordinate="{\"name\":\"Nonfinite\",\"address\":{\"display_name\":\"Synthetic\",\"latitude\":1e309,\"longitude\":0}}";
    let response = request(&state, 1, "POST", "/v1/places", raw_coordinate.into()).await;
    e.record(&state, "nonfinite-coordinate-json", response)
        .await;
    let full = call(
        &mut e,
        &state,
        "full-edit-seed",
        1,
        "POST",
        "/v1/cost-profiles",
        json!({"name":"Before edit","billing_type":"per_kwh","rate":0.12}),
    )
    .await;
    let full_path = format!("/v1/cost-profiles/{}", full["id"].as_str().unwrap());
    let change = json!({"name":"After edit","billing_type":"per_minute","rate":0.5,"session_fee":2.0,"currency":"CAD","timezone":"America/Los_Angeles","tou_periods":[{"kept":"opaque for non-TOU"}],"effective_from":"2026-01-02","effective_to":"2026-01-03"});
    call(
        &mut e,
        &state,
        "full-profile-edit",
        1,
        "PUT",
        &full_path,
        change.clone(),
    )
    .await;
    let exact_before: Value =
        sqlx::query_scalar("SELECT to_jsonb(p) FROM panda.cost_profiles p WHERE id=$1")
            .bind(Uuid::parse_str(full["id"].as_str().unwrap()).unwrap())
            .fetch_one(pool)
            .await
            .unwrap();
    call(
        &mut e,
        &state,
        "full-profile-repeat",
        1,
        "PUT",
        &full_path,
        change,
    )
    .await;
    let exact_after: Value =
        sqlx::query_scalar("SELECT to_jsonb(p) FROM panda.cost_profiles p WHERE id=$1")
            .bind(Uuid::parse_str(full["id"].as_str().unwrap()).unwrap())
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(exact_before, exact_after);
    call(&mut e,&state,"full-profile-null-inherit",1,"PUT",&full_path,json!({"name":null,"billing_type":null,"rate":null,"session_fee":null,"currency":null,"timezone":null,"tou_periods":null,"effective_from":null,"effective_to":null})).await;
    call(
        &mut e,
        &state,
        "foreign-invalid-profile-owner-check-first",
        2,
        "PUT",
        &full_path,
        json!({"billing_type":"invalid","rate":-1}),
    )
    .await;
    call(
        &mut e,
        &state,
        "full-profile-invalid-edit",
        1,
        "PUT",
        &full_path,
        json!({"billing_type":"per_kwh","rate":5.0001}),
    )
    .await;
    call(
        &mut e,
        &state,
        "full-profile-list",
        1,
        "GET",
        "/v1/cost-profiles",
        Value::Null,
    )
    .await;
    call(
        &mut e,
        &state,
        "full-profile-delete",
        1,
        "DELETE",
        &full_path,
        Value::Null,
    )
    .await;
    let events = e.events;
    let raw_events = e.raw;
    db.cleanup().await;
    let raw = serde_json::to_string_pretty(&events).unwrap();
    if let Some(dir) = std::env::var_os("PLACES_EVIDENCE_DIR") {
        std::fs::write(std::path::Path::new(&dir).join("http-actual.json"), &raw).unwrap();
        std::fs::write(
            std::path::Path::new(&dir).join("http-raw.json"),
            serde_json::to_string_pretty(&raw_events).unwrap(),
        )
        .unwrap();
    }
    if std::env::var_os("PLACES_CAPTURE").is_none() {
        let expected: Value = serde_json::from_str(include_str!("http.json")).unwrap();
        assert_eq!(json!(events), expected);
    }
}
