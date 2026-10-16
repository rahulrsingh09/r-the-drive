//! Fixed synthetic characterization of the public cost and persistence contracts.
use super::*;
use crate::models::cost_profile::{validate_tou_periods, TouPeriod};
use chrono::{Duration, NaiveDate};
use serde_json::{json, Value};

fn instant(value: &str) -> DateTime<Utc> {
    value.parse().unwrap()
}

fn id(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

fn profile() -> CostProfile {
    CostProfile {
        id: id(100),
        user_id: id(1),
        name: "Synthetic tariff".into(),
        billing_type: "per_kwh".into(),
        rate: 0.25,
        session_fee: 1.5,
        currency: "CAD".into(),
        timezone: Some("UTC".into()),
        tou_periods: json!([
            {"label":"Night", "start_minute":0, "end_minute":360, "rate":0.1},
            {"label":"Day", "start_minute":360, "end_minute":1200, "rate":0.4},
            {"label":"Evening", "start_minute":1200, "end_minute":1440, "rate":0.0}
        ]),
        effective_from: None,
        effective_to: None,
        created_at: instant("2026-01-01T00:00:00Z"),
    }
}

fn inputs() -> CostInputs {
    CostInputs {
        is_rivian_network: None,
        is_home: None,
        is_free_session: None,
        network_cost_mode: None,
        cost_override_mode: "automatic".into(),
        cost_override_usd: Some(7.75),
        rivian_paid_total: Some(9.25),
        energy_added_kwh: Some(10.0),
        energy_used_kwh: Some(12.0),
        duration_minutes: 120,
        started_at: instant("2026-01-01T05:00:00Z"),
        ended_at: Some(instant("2026-01-01T07:00:00Z")),
        timed_kwh_readings: vec![],
    }
}

fn number(value: Option<f64>) -> Value {
    match value {
        None => Value::Null,
        Some(value) => json!({
            "value": if value.is_nan() { json!("NaN") }
                else if value == f64::INFINITY { json!("+Infinity") }
                else if value == f64::NEG_INFINITY { json!("-Infinity") }
                else { json!(value) },
            "bits": format!("{:016x}", value.to_bits()),
        }),
    }
}

fn computation(result: &ChargeCostComputation) -> Value {
    json!({
        "cost_profile_id": result.cost_profile_id,
        "cost_method": result.cost_method,
        "cost_usd": number(result.cost_usd),
    })
}

fn assert_snapshot(name: &str, rows: &[Value], expected: &str) {
    if let Ok(directory) = std::env::var("COST_CONTRACT_ARTIFACT_DIR") {
        std::fs::write(
            std::path::Path::new(&directory).join(format!("cost-{name}-actual.json")),
            serde_json::to_string_pretty(rows).unwrap() + "\n",
        )
        .unwrap();
    }
    let mut expected: Vec<Value> = serde_json::from_str(expected).unwrap();
    if name == "database" {
        let owner_default = expected
            .iter()
            .find(|row| row["name"] == "owner default")
            .unwrap()["profile"]
            .clone();
        let mut corrected = 0;
        for row in &mut expected {
            if matches!(
                row["name"].as_str(),
                Some(
                    "linked foreign geofence trusted by seam"
                        | "explicit foreign profile trusted by seam"
                )
            ) {
                assert_eq!(row["profile"]["user_id"], id(2).to_string());
                row["profile"] = owner_default.clone();
                corrected += 1;
            }
        }
        assert_eq!(corrected, 2);
    }
    assert_eq!(rows.len(), expected.len(), "{name} snapshot row count");
    for (index, (actual, expected)) in rows.iter().zip(expected.iter()).enumerate() {
        assert_eq!(actual, expected, "{name} complete row {index}");
    }
}

#[test]
fn cost_contracts_pure_matrix() {
    let mut rows = vec![];
    let p = profile();
    rows.push(json!({"case":"profile serialization", "profile":p}));
    let mut extended = serde_json::to_value(&p).unwrap();
    extended["future_field"] = json!(true);
    let decoded: CostProfile = serde_json::from_value(extended).unwrap();
    rows.push(json!({"case":"unknown field roundtrip", "profile":decoded}));
    for override_mode in ["automatic", "manual", "free", "unknown"] {
        for network_mode in [None, Some("automatic"), Some("free"), Some("unknown")] {
            for is_free in [None, Some(false), Some(true)] {
                for is_network in [None, Some(false), Some(true)] {
                    for is_home in [None, Some(false), Some(true)] {
                        let mut i = inputs();
                        i.cost_override_mode = override_mode.into();
                        i.network_cost_mode = network_mode.map(str::to_owned);
                        i.is_free_session = is_free;
                        i.is_rivian_network = is_network;
                        i.is_home = is_home;
                        rows.push(json!({
                            "case":"precedence", "override":override_mode, "network_mode":network_mode,
                            "free":is_free, "network":is_network, "home":is_home,
                            "output":computation(&apply_cost_inputs(&i, Some(&p))),
                            "without_profile":computation(&apply_cost_inputs(&i, None)),
                        }));
                    }
                }
            }
        }
    }
    for mode in ["manual", "automatic"] {
        for amount in [
            None,
            Some(0.0),
            Some(-0.0),
            Some(-2.0),
            Some(f64::NAN),
            Some(f64::INFINITY),
        ] {
            let mut i = inputs();
            i.cost_override_mode = mode.into();
            i.cost_override_usd = amount;
            i.rivian_paid_total = amount;
            i.is_rivian_network = Some(true);
            rows.push(
                json!({"case":"amount", "mode":mode, "amount":number(amount),
                "output":computation(&apply_cost_inputs(&i, Some(&p)))}),
            );
        }
    }
    let energy = [
        None,
        Some(-2.0),
        Some(0.0),
        Some(-0.0),
        Some(10.0),
        Some(f64::NAN),
        Some(f64::INFINITY),
    ];
    for billing in ["per_kwh", "per_minute", "free", "flat", "tou", "unknown"] {
        for added in energy {
            for used in energy {
                let mut p = profile();
                p.billing_type = billing.into();
                let mut i = inputs();
                i.energy_added_kwh = added;
                i.energy_used_kwh = used;
                rows.push(json!({"case":"energy", "billing":billing, "added":number(added), "used":number(used),
                    "model":number(compute_cost(&p, added, used, i.duration_minutes, i.started_at, i.ended_at)),
                    "output":computation(&apply_cost_inputs(&i, Some(&p)))}));
            }
        }
        for duration in [-60, 0, 1, 60] {
            let mut p = profile();
            p.billing_type = billing.into();
            for ended in [
                None,
                Some(instant("2026-01-01T04:00:00Z")),
                Some(instant("2026-01-01T05:00:00Z")),
            ] {
                let mut i = inputs();
                i.duration_minutes = duration;
                i.ended_at = ended;
                rows.push(json!({"case":"duration", "billing":billing, "duration":duration, "ended_at":ended,
                    "output":computation(&apply_cost_inputs(&i, Some(&p)))}));
            }
        }
    }
    for tz in [
        None,
        Some("UTC"),
        Some("America/Chicago"),
        Some("invalid/timezone"),
    ] {
        for start in [
            "2026-01-01T00:30:00Z",
            "2026-01-02T00:30:00Z",
            "2026-01-03T06:30:00Z",
        ] {
            for window in [
                (None, None),
                (Some("2026-01-02"), None),
                (None, Some("2026-01-02")),
                (Some("2026-01-02"), Some("2026-01-02")),
            ] {
                let mut p = profile();
                p.timezone = tz.map(str::to_owned);
                p.effective_from = window.0.map(|v| v.parse::<NaiveDate>().unwrap());
                p.effective_to = window.1.map(|v| v.parse::<NaiveDate>().unwrap());
                let mut i = inputs();
                i.started_at = instant(start);
                i.ended_at = Some(i.started_at + Duration::hours(2));
                p.billing_type = "tou".into();
                let without_readings = computation(&apply_cost_inputs(&i, Some(&p)));
                i.timed_kwh_readings = vec![TimedEnergyPoint {
                    ts: i.started_at,
                    kwh: 5.0,
                }];
                rows.push(
                    json!({"case":"effective dates", "timezone":tz, "start":start, "window":window,
                    "effective":p.is_effective_at(i.started_at), "time_weighted":without_readings,
                    "measured":computation(&apply_cost_inputs(&i, Some(&p)))}),
                );
            }
        }
    }
    for (name, tz, start, end, periods) in [
        (
            "spring forward",
            "America/Chicago",
            "2026-03-08T06:00:00Z",
            "2026-03-09T05:00:00Z",
            json!([
            {"label":"first","start_minute":0,"end_minute":120,"rate":0.1},
            {"label":"gap","start_minute":120,"end_minute":180,"rate":0.2},
            {"label":"rest","start_minute":180,"end_minute":1440,"rate":0.4}]),
        ),
        (
            "fall back",
            "America/Chicago",
            "2026-11-01T05:00:00Z",
            "2026-11-02T06:00:00Z",
            json!([
            {"label":"first","start_minute":0,"end_minute":60,"rate":0.1},
            {"label":"repeated","start_minute":60,"end_minute":120,"rate":0.2},
            {"label":"rest","start_minute":120,"end_minute":1440,"rate":0.4}]),
        ),
        (
            "multi day",
            "Asia/Kolkata",
            "2026-01-01T17:45:13Z",
            "2026-01-03T22:01:29Z",
            p.tou_periods.clone(),
        ),
        (
            "overlap first match",
            "UTC",
            "2026-01-01T05:00:00Z",
            "2026-01-01T07:00:00Z",
            json!([
            {"label":"a","start_minute":0,"end_minute":1440,"rate":0.1},
            {"label":"b","start_minute":0,"end_minute":1440,"rate":0.9}]),
        ),
        (
            "missing interval",
            "UTC",
            "2026-01-01T05:00:00Z",
            "2026-01-01T07:00:00Z",
            json!([
            {"label":"a","start_minute":0,"end_minute":60,"rate":0.1}]),
        ),
        (
            "clamped end",
            "UTC",
            "2026-01-01T23:00:00Z",
            "2026-01-02T01:00:00Z",
            json!([
            {"label":"a","start_minute":0,"end_minute":1600,"rate":0.1}]),
        ),
        (
            "malformed",
            "UTC",
            "2026-01-01T05:00:00Z",
            "2026-01-01T07:00:00Z",
            json!([{"label":"broken"}]),
        ),
        (
            "null periods",
            "UTC",
            "2026-01-01T05:00:00Z",
            "2026-01-01T07:00:00Z",
            Value::Null,
        ),
    ] {
        let mut p = profile();
        p.billing_type = "tou".into();
        p.timezone = Some(tz.into());
        p.tou_periods = periods;
        let mut i = inputs();
        i.started_at = instant(start);
        i.ended_at = Some(instant(end));
        let weighted = computation(&apply_cost_inputs(&i, Some(&p)));
        for (reading_name, values) in [
            ("empty", vec![]),
            ("boundaries", vec![(start, 1.0), (end, 3.0)]),
            (
                "out of window unsorted",
                vec![
                    ("2026-01-04T06:00:00Z", 4.0),
                    ("2026-01-01T05:59:59Z", 2.0),
                    ("2026-01-01T06:00:00Z", 1.0),
                ],
            ),
            ("nonpositive", vec![(start, -2.0), (end, 0.0)]),
            ("nonfinite", vec![(start, f64::NAN), (end, f64::INFINITY)]),
        ] {
            i.timed_kwh_readings = values
                .into_iter()
                .map(|(ts, kwh)| TimedEnergyPoint {
                    ts: instant(ts),
                    kwh,
                })
                .collect();
            rows.push(
                json!({"case":"tou intervals", "name":name, "readings":reading_name,
                "periods":p.tou_periods(), "time_weighted":weighted,
                "readings_only":number(compute_tou_cost_from_readings(&p,&i.timed_kwh_readings)),
                "output":computation(&apply_cost_inputs(&i,Some(&p)))}),
            );
        }
    }
    for (name, periods) in [
        ("empty", vec![]),
        (
            "blank before bad rate",
            vec![TouPeriod {
                label: " ".into(),
                start_minute: 4,
                end_minute: 0,
                rate: f64::NAN,
            }],
        ),
        (
            "nan",
            vec![TouPeriod {
                label: "a".into(),
                start_minute: 0,
                end_minute: 1440,
                rate: f64::NAN,
            }],
        ),
        (
            "infinity",
            vec![TouPeriod {
                label: "a".into(),
                start_minute: 0,
                end_minute: 1440,
                rate: f64::INFINITY,
            }],
        ),
        (
            "negative",
            vec![TouPeriod {
                label: "a".into(),
                start_minute: 0,
                end_minute: 1440,
                rate: -0.1,
            }],
        ),
        (
            "cents before gap",
            vec![TouPeriod {
                label: "a".into(),
                start_minute: 4,
                end_minute: 1440,
                rate: 32.2499,
            }],
        ),
        (
            "limit",
            vec![TouPeriod {
                label: "a".into(),
                start_minute: 0,
                end_minute: 1440,
                rate: 5.0,
            }],
        ),
        (
            "gap before reversed",
            vec![TouPeriod {
                label: "a".into(),
                start_minute: 4,
                end_minute: 0,
                rate: 0.1,
            }],
        ),
        (
            "reversed",
            vec![TouPeriod {
                label: "a".into(),
                start_minute: 0,
                end_minute: 0,
                rate: 0.1,
            }],
        ),
        (
            "incomplete",
            vec![TouPeriod {
                label: "a".into(),
                start_minute: 0,
                end_minute: 1400,
                rate: 0.1,
            }],
        ),
        (
            "overshoot",
            vec![TouPeriod {
                label: "a".into(),
                start_minute: 0,
                end_minute: 1500,
                rate: 0.1,
            }],
        ),
        (
            "negative zero",
            vec![TouPeriod {
                label: "a".into(),
                start_minute: 0,
                end_minute: 1440,
                rate: -0.0,
            }],
        ),
        ("valid", p.tou_periods()),
    ] {
        rows.push(
            json!({"case":"validation", "name":name, "result":validate_tou_periods(&periods)}),
        );
    }
    assert_snapshot("pure", &rows, include_str!("cost_contracts/pure.json"));
}

struct DatabaseFixture {
    admin: PgPool,
    pool: PgPool,
    database: String,
    role: String,
}

impl DatabaseFixture {
    async fn new() -> Self {
        use sqlx::Executor;
        let mut url = url::Url::parse(
            &std::env::var("COST_CONTRACT_DATABASE_URL")
                .expect("explicit disposable COST_CONTRACT_DATABASE_URL required"),
        )
        .unwrap();
        assert_eq!(url.host_str(), Some("db"));
        assert_eq!(url.username(), "cost_fixture");
        assert_eq!(url.path(), "/cost_fixture");
        url.set_path("/postgres");
        let admin = PgPool::connect(url.as_str()).await.unwrap();
        let suffix = Uuid::new_v4().simple().to_string();
        let role = format!("cost_role_{suffix}");
        let database = format!("cost_contract_{suffix}");
        admin.execute(sqlx::AssertSqlSafe(format!(
            "CREATE ROLE {role} LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOREPLICATION NOBYPASSRLS PASSWORD 'synthetic_cost_app'"
        ))).await.unwrap();
        admin
            .execute(sqlx::AssertSqlSafe(format!(
                "CREATE DATABASE {database} OWNER {role}"
            )))
            .await
            .unwrap();
        url.set_path(&format!("/{database}"));
        let bootstrap = PgPool::connect(url.as_str()).await.unwrap();
        bootstrap
            .execute(sqlx::AssertSqlSafe(format!(
                "CREATE SCHEMA panda AUTHORIZATION {role};
             REVOKE CREATE ON SCHEMA public FROM PUBLIC;
             REVOKE ALL ON SCHEMA panda FROM PUBLIC;
             CREATE EXTENSION IF NOT EXISTS pgcrypto WITH SCHEMA public;
             CREATE EXTENSION IF NOT EXISTS timescaledb WITH SCHEMA public;
             CREATE EXTENSION IF NOT EXISTS cube WITH SCHEMA panda;
             CREATE EXTENSION IF NOT EXISTS earthdistance WITH SCHEMA panda;"
            )))
            .await
            .unwrap();
        bootstrap.close().await;
        url.set_username(&role).unwrap();
        url.set_password(Some("synthetic_cost_app")).unwrap();
        let migration_pool = PgPool::connect(url.as_str()).await.unwrap();
        crate::db::migrations::run_current_migrations(&migration_pool)
            .await
            .unwrap();
        migration_pool.close().await;
        let pool = crate::db::pool::application_pool_options()
            .max_connections(2)
            .connect(url.as_str())
            .await
            .unwrap();
        let config = serde_json::from_value(json!({
            "database_url":url.as_str(), "redis_url":"redis://unused", "panda_env":"production"
        }))
        .unwrap();
        crate::private_deployment::history::protect_history(&pool, &config)
            .await
            .unwrap();
        Self {
            admin,
            pool,
            database,
            role,
        }
    }

    async fn cleanup(self) {
        use sqlx::Executor;
        self.pool.close().await;
        self.admin
            .execute(sqlx::AssertSqlSafe(format!(
                "DROP DATABASE {} WITH (FORCE)",
                self.database
            )))
            .await
            .unwrap();
        self.admin
            .execute(sqlx::AssertSqlSafe(format!("DROP ROLE {}", self.role)))
            .await
            .unwrap();
        self.admin.close().await;
    }
}

async fn seed_database(pool: &PgPool) {
    for n in 1..=3 {
        sqlx::query("INSERT INTO panda.users (id,email,password_hash,created_at,updated_at) VALUES ($1,$2,'inert fixture', '2026-01-01Z','2026-01-01Z')")
            .bind(id(n)).bind(format!("cost-{n}@example.invalid")).execute(pool).await.unwrap();
    }
    for n in [10, 11] {
        sqlx::query("INSERT INTO panda.vehicles (id,user_id,rivian_vehicle_id,model,created_at,updated_at) VALUES ($1,$2,$3,'synthetic','2026-01-01Z','2026-01-01Z')")
            .bind(id(n)).bind(id(1)).bind(format!("synthetic-cost-{n}")).execute(pool).await.unwrap();
    }
    for (n, user, role, created) in [
        (30, 1, "owner", "2026-01-01T00:00:00Z"),
        (31, 2, "viewer", "2026-01-01T00:01:00Z"),
        (32, 3, "owner", "2026-01-01T00:02:00Z"),
    ] {
        sqlx::query("INSERT INTO panda.vehicle_memberships (id,vehicle_id,user_id,role,created_at,updated_at) VALUES ($1,$2,$3,$4,$5,$5)")
            .bind(id(n)).bind(id(10)).bind(id(user)).bind(role).bind(instant(created)).execute(pool).await.unwrap();
    }
    for (n, user, billing, rate) in [
        (100, 1, "flat", 10.0),
        (101, 1, "per_kwh", 0.25),
        (102, 1, "per_minute", 0.1),
        (103, 2, "flat", 99.0),
        (104, 1, "flat", 999.0),
        (105, 1, "tou", 0.0),
    ] {
        let mut p = profile();
        p.id = id(n);
        p.user_id = id(user);
        p.billing_type = billing.into();
        p.rate = rate;
        if n == 104 {
            p.effective_to = Some("2025-12-31".parse().unwrap());
        }
        sqlx::query("INSERT INTO panda.cost_profiles (id,user_id,name,billing_type,rate,session_fee,currency,timezone,tou_periods,effective_from,effective_to,created_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)")
            .bind(p.id).bind(p.user_id).bind(&p.name).bind(&p.billing_type).bind(p.rate).bind(p.session_fee)
            .bind(&p.currency).bind(&p.timezone).bind(&p.tou_periods).bind(p.effective_from).bind(p.effective_to).bind(p.created_at)
            .execute(pool).await.unwrap();
    }
    for (n, user, p) in [(20, 1, 101), (21, 2, 103), (22, 1, 104)] {
        sqlx::query("INSERT INTO panda.geofences (id,user_id,name,latitude,longitude,cost_profile_id,created_at,updated_at) VALUES ($1,$2,'synthetic',0,0,$3,'2026-01-01Z','2026-01-01Z')")
            .bind(id(n)).bind(id(user)).bind(id(p)).execute(pool).await.unwrap();
    }
    for (user, p) in [(1, 102), (2, 103), (3, 100)] {
        sqlx::query("INSERT INTO panda.vehicle_user_settings (vehicle_id,user_id,default_cost_profile_id,created_at,updated_at) VALUES ($1,$2,$3,'2026-01-01Z','2026-01-01Z')")
            .bind(id(10)).bind(id(user)).bind(id(p)).execute(pool).await.unwrap();
    }
    sqlx::query("INSERT INTO panda.vehicle_charging_network_preferences (vehicle_id,vendor_normalized,network_vendor,cost_mode) VALUES ($1,'synthetic free network','Synthetic Free Network','free')")
        .bind(id(10)).execute(pool).await.unwrap();
}

async fn persisted_session(pool: &PgPool, session: Uuid) -> Value {
    let row: Value =
        sqlx::query_scalar("SELECT to_jsonb(cs) FROM panda.charge_sessions cs WHERE id=$1")
            .bind(session)
            .fetch_one(pool)
            .await
            .unwrap();
    let annotations: Vec<Value> = sqlx::query_scalar(
        "SELECT to_jsonb(a) FROM panda.charge_session_user_annotations a WHERE charge_session_id=$1 ORDER BY user_id"
    ).bind(session).fetch_all(pool).await.unwrap();
    json!({"session":row, "annotations":annotations})
}

fn normalize_clock(value: &mut Value) {
    match value {
        Value::Object(fields) => {
            for (name, value) in fields {
                if ["computed_at", "created_at", "updated_at"].contains(&name.as_str())
                    && value.is_string()
                {
                    let timestamp = value.as_str().unwrap();
                    if !timestamp.starts_with("2026-01-01T00:00:00") {
                        *value = json!("<database clock>");
                    }
                } else {
                    normalize_clock(value);
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                normalize_clock(value);
            }
        }
        _ => {}
    }
}

async fn database_matrix(pool: &PgPool) -> Vec<Value> {
    use sqlx::Executor;
    seed_database(pool).await;
    let mut rows = vec![];
    let restricted: Value = sqlx::query_scalar(
        "SELECT jsonb_build_object('superuser',rolsuper,'createdb',rolcreatedb,'createrole',rolcreaterole,
          'bypassrls',rolbypassrls,'telemetry_delete',has_table_privilege(current_user,'timeseries.telemetry','DELETE'),
          'vehicle_delete',has_table_privilege(current_user,'panda.vehicles','DELETE'),
          'charge_truncate',has_table_privilege(current_user,'panda.charge_sessions','TRUNCATE'))
         FROM pg_roles WHERE rolname=current_user"
    ).fetch_one(pool).await.unwrap();
    assert_eq!(
        restricted,
        json!({"superuser":false,"createdb":false,"createrole":false,"bypassrls":false,
        "telemetry_delete":false,"vehicle_delete":false,"charge_truncate":false})
    );
    rows.push(json!({"case":"history privileges","output":restricted}));
    for (name, explicit, geofence, vehicle) in [
        ("pinned", Some(100), Some(20), 10),
        ("missing pinned", Some(999), Some(20), 10),
        ("expired pinned", Some(104), Some(20), 10),
        ("geofence", None, Some(20), 10),
        ("expired geofence", None, Some(22), 10),
        ("missing geofence", None, Some(999), 10),
        ("owner default", None, None, 10),
        ("no membership owner", None, None, 11),
        (
            "linked foreign geofence trusted by seam",
            None,
            Some(21),
            10,
        ),
        (
            "explicit foreign profile trusted by seam",
            Some(103),
            None,
            10,
        ),
    ] {
        let output = resolve_profile(
            pool,
            explicit.map(id),
            geofence.map(id),
            id(vehicle),
            instant("2026-01-02T00:00:00Z"),
        )
        .await
        .unwrap();
        rows.push(json!({"case":"resolution","name":name,"profile":output}));
    }
    rows.push(json!({"case":"missing session",
        "output":recompute_charge_session_cost(pool,id(999)).await.unwrap().as_ref().map(computation)}));
    for (
        n,
        name,
        profile_id,
        geofence,
        vehicle,
        override_mode,
        override_usd,
        network,
        home,
        free,
        vendor,
        paid,
        added_wh,
        used_wh,
        duration,
        ended,
    ) in [
        (
            200,
            "geofence wh energy",
            None,
            Some(20),
            10,
            "automatic",
            None,
            None,
            Some(true),
            None,
            None,
            None,
            Some(12000.0),
            Some(14000.0),
            Some(60),
            true,
        ),
        (
            201,
            "pinned flat",
            Some(100),
            Some(20),
            10,
            "automatic",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(60),
            true,
        ),
        (
            202,
            "owner derived minutes",
            None,
            None,
            10,
            "automatic",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            true,
        ),
        (
            203,
            "owner stored negative duration",
            None,
            None,
            10,
            "automatic",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(-5),
            true,
        ),
        (
            204,
            "manual null",
            Some(101),
            Some(20),
            10,
            "manual",
            None,
            Some(true),
            None,
            Some(true),
            Some("Synthetic Free Network"),
            Some(30.0),
            None,
            None,
            None,
            true,
        ),
        (
            205,
            "manual amount",
            Some(101),
            Some(20),
            10,
            "manual",
            Some(8.5),
            Some(true),
            None,
            Some(true),
            None,
            Some(30.0),
            None,
            None,
            None,
            true,
        ),
        (
            206,
            "user free",
            Some(101),
            None,
            10,
            "free",
            None,
            Some(true),
            None,
            None,
            None,
            Some(30.0),
            None,
            None,
            None,
            true,
        ),
        (
            207,
            "network normalized free",
            Some(101),
            None,
            10,
            "automatic",
            None,
            None,
            None,
            None,
            Some("  SYNTHETIC FREE NETWORK  "),
            Some(30.0),
            None,
            None,
            None,
            true,
        ),
        (
            208,
            "provider free",
            Some(101),
            None,
            10,
            "automatic",
            None,
            None,
            None,
            Some(true),
            None,
            Some(30.0),
            None,
            None,
            None,
            true,
        ),
        (
            209,
            "paid network retains profile",
            Some(101),
            None,
            10,
            "automatic",
            None,
            Some(true),
            Some(false),
            None,
            None,
            Some(30.0),
            None,
            None,
            None,
            true,
        ),
        (
            210,
            "home rejects paid",
            Some(101),
            None,
            10,
            "automatic",
            None,
            Some(true),
            Some(true),
            None,
            None,
            Some(30.0),
            None,
            None,
            None,
            true,
        ),
        (
            211,
            "no owner",
            None,
            None,
            11,
            "automatic",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            true,
        ),
        (
            212,
            "pending zero energy",
            Some(101),
            None,
            10,
            "automatic",
            None,
            None,
            None,
            None,
            None,
            None,
            Some(0.0),
            Some(0.0),
            Some(60),
            true,
        ),
        (
            213,
            "tou weighted",
            Some(105),
            None,
            10,
            "automatic",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            true,
        ),
        (
            214,
            "tou curve",
            Some(105),
            None,
            10,
            "automatic",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            true,
        ),
        (
            215,
            "tou telemetry wins",
            Some(105),
            None,
            10,
            "automatic",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            true,
        ),
        (
            216,
            "open tou skips readings",
            Some(105),
            None,
            10,
            "automatic",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(120),
            false,
        ),
    ] {
        let start = instant("2026-01-01T05:00:00Z") + Duration::days(n - 200);
        let end = ended.then_some(start + Duration::hours(2));
        let inserted = sqlx::query(
            "INSERT INTO panda.charge_sessions
             (id,vehicle_id,started_at,ended_at,kwh_added,energy_added_wh,energy_used_wh,duration_minutes,
              cost_profile_id,geofence_id,cost_override_mode,cost_override_usd,is_rivian_network,is_home,
              is_free_session,network_vendor,rivian_paid_total,currency_code,created_at)
             VALUES ($1,$2,$3,$4,10,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,'CAD','2026-01-01Z')"
        ).bind(id(n as u128)).bind(id(vehicle)).bind(start).bind(end).bind(added_wh).bind(used_wh).bind(duration)
            .bind(profile_id.map(id)).bind(geofence.map(id)).bind(override_mode).bind(override_usd)
            .bind(network).bind(home).bind(free).bind(vendor).bind(paid).execute(pool).await;
        if n == 204 {
            let error = inserted.unwrap_err();
            let db = error.as_database_error().unwrap();
            assert_eq!(db.code().as_deref(), Some("23514"));
            rows.push(
                json!({"case":"manual null rejected by schema","code":db.code(),
                "message":db.message(),"constraint":db.constraint()}),
            );
            continue;
        }
        inserted.unwrap();
        // The viewer's annotations and the owner's preexisting location must survive.
        for user in [1, 2] {
            sqlx::query(
                "INSERT INTO panda.charge_session_user_annotations
                 (charge_session_id,user_id,geofence_id,is_home,cost_profile_id,cost_method,cost_usd,currency_code,computed_at,created_at,updated_at)
                 VALUES ($1,$2,$3,true,$4,'fixture',42,'EUR','2026-01-01Z','2026-01-01Z','2026-01-01Z')"
            ).bind(id(n as u128)).bind(id(user)).bind(id(20)).bind(id(100)).execute(pool).await.unwrap();
        }
        if [214, 215, 216].contains(&n) {
            for (minute, power) in [(0, 60.0), (60, -3.0), (90, 120.0)] {
                sqlx::query("INSERT INTO panda.rivian_charge_curve_points (vehicle_id,charge_session_id,ts,power_kw,captured_at) VALUES ($1,$2,$3,$4,'2026-01-01Z')")
                    .bind(id(vehicle)).bind(id(n as u128)).bind(start+Duration::minutes(minute)).bind(power).execute(pool).await.unwrap();
            }
        }
        if n == 215 {
            for (minute, soc, capacity) in [
                (0, 50.0, 100000.0),
                (59, 52.0, 110000.0),
                (60, 51.0, 100000.0),
                (61, 54.0, 100000.0),
            ] {
                sqlx::query("INSERT INTO timeseries.telemetry (ts,vehicle_id,battery_level,battery_capacity_wh,charge_session_id) VALUES ($1,$2,$3,$4,$5)")
                    .bind(start+Duration::minutes(minute)).bind(id(vehicle)).bind(soc).bind(capacity).bind(id(n as u128))
                    .execute(pool).await.unwrap();
            }
            sqlx::query("CALL refresh_continuous_aggregate('timeseries.telemetry_1min',$1::timestamptz,$2::timestamptz)")
                .bind(start).bind(start+Duration::hours(2)).execute(pool).await.unwrap();
        }
        let readings = if let Some(end) = end {
            fetch_session_energy_readings(pool, id(vehicle), start, end, id(n as u128))
                .await
                .unwrap()
        } else {
            vec![]
        };
        let before = persisted_session(pool, id(n as u128)).await;
        let output = recompute_charge_session_cost(pool, id(n as u128))
            .await
            .unwrap()
            .unwrap();
        let first = persisted_session(pool, id(n as u128)).await;
        let second_output = recompute_charge_session_cost(pool, id(n as u128))
            .await
            .unwrap()
            .unwrap();
        let second = persisted_session(pool, id(n as u128)).await;
        assert_eq!(output, second_output, "{name} result idempotence");
        assert_eq!(
            first, second,
            "{name} complete persisted row idempotence including clocks"
        );
        assert_eq!(
            before["annotations"][1], second["annotations"][1],
            "{name} viewer isolation"
        );
        if vehicle == 11 {
            assert_eq!(before["annotations"], second["annotations"]);
        }
        rows.push(json!({"case":"recompute","name":name,"output":computation(&output),
            "readings":readings.iter().map(|r|json!({"ts":r.ts,"kwh":number(Some(r.kwh))})).collect::<Vec<_>>(),
            "before":before,"after":first}));
    }
    pool.execute("REVOKE SELECT ON timeseries.telemetry_1min FROM CURRENT_USER")
        .await
        .unwrap();
    let error = fetch_session_energy_readings(
        pool,
        id(10),
        instant("2026-01-16T05:00:00Z"),
        instant("2026-01-16T07:00:00Z"),
        id(215),
    )
    .await
    .unwrap_err();
    let output = recompute_charge_session_cost(pool, id(215))
        .await
        .unwrap()
        .unwrap();
    rows.push(json!({"case":"unavailable telemetry falls back to time weighting",
        "error":error.to_string(),"output":computation(&output),"after":persisted_session(pool,id(215)).await}));
    pool.execute("GRANT SELECT ON timeseries.telemetry_1min TO CURRENT_USER")
        .await
        .unwrap();
    pool.execute("REVOKE UPDATE ON panda.charge_session_user_annotations FROM CURRENT_USER")
        .await
        .unwrap();
    sqlx::query("UPDATE panda.charge_sessions SET cost_override_mode='manual',cost_override_usd=123 WHERE id=$1")
        .bind(id(200)).execute(pool).await.unwrap();
    let error = recompute_charge_session_cost(pool, id(200))
        .await
        .unwrap_err();
    rows.push(json!({"case":"annotation failure after canonical update",
        "error":error.to_string(),"after":persisted_session(pool,id(200)).await}));
    pool.execute("GRANT UPDATE ON panda.charge_session_user_annotations TO CURRENT_USER")
        .await
        .unwrap();
    let output = recompute_charge_session_cost(pool, id(200))
        .await
        .unwrap()
        .unwrap();
    rows.push(json!({"case":"annotation failure retry",
        "output":computation(&output),"after":persisted_session(pool,id(200)).await}));
    for sql in ["DELETE FROM timeseries.telemetry WHERE vehicle_id = '00000000-0000-0000-0000-00000000000a'",
                "DELETE FROM panda.vehicles WHERE id = '00000000-0000-0000-0000-00000000000b'",
                "TRUNCATE panda.charge_sessions"] {
        let error = pool.execute(sql).await.unwrap_err();
        assert_eq!(error.as_database_error().unwrap().code().as_deref(),Some("42501"));
        rows.push(json!({"case":"protected write","query":sql,
            "code":error.as_database_error().unwrap().code(),"message":error.as_database_error().unwrap().message()}));
    }
    rows
}

#[tokio::test]
#[ignore = "requires the unique disposable cost-engine Compose database; run explicitly"]
async fn cost_contracts_database_matrix() {
    let fixture = DatabaseFixture::new().await;
    let mut rows = database_matrix(&fixture.pool).await;
    if let Ok(directory) = std::env::var("COST_CONTRACT_ARTIFACT_DIR") {
        std::fs::write(
            std::path::Path::new(&directory).join("cost-database-raw.json"),
            serde_json::to_string_pretty(&rows).unwrap(),
        )
        .unwrap();
    }
    fixture.cleanup().await;
    for row in &mut rows {
        normalize_clock(row);
    }
    assert_snapshot(
        "database",
        &rows,
        include_str!("cost_contracts/database.json"),
    );
}
