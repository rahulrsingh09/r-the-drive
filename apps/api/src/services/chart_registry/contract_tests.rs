use super::*;
use serde_json::json;
use sqlx::{postgres::PgPoolOptions, Executor};

fn compare(name: &str, actual: &Value, expected: &str) {
    if let Ok(directory) = std::env::var("CHART_CONTRACT_CAPTURE") {
        std::fs::write(
            format!("{directory}/{name}-expected.json"),
            serde_json::to_string_pretty(actual).unwrap(),
        )
        .unwrap();
    } else {
        assert_eq!(
            *actual,
            serde_json::from_str::<Value>(expected).unwrap(),
            "{name}"
        );
    }
}

fn record(index: u128, slug: &str, owner: Option<Uuid>, locked: bool) -> ChartRecord {
    ChartRecord {
        id: Uuid::from_u128(index),
        owner_id: owner,
        slug: slug.into(),
        name: format!("Synthetic {index}"),
        description: if index % 2 == 0 {
            None
        } else {
            Some("Fixture".into())
        },
        is_default: owner.is_none(),
        is_locked: locked,
        is_enabled: index % 2 == 0,
        config: json!({"synthetic": index.to_string()}),
        baseline_revision: owner.is_none().then_some(5),
        created_at: "2026-01-01T00:00:00Z".parse().unwrap(),
        updated_at: "2026-01-02T00:00:00Z".parse().unwrap(),
    }
}

#[test]
fn complete_manager_resolution_contract() {
    let user = Uuid::from_u128(100);
    let other = Uuid::from_u128(101);
    let candidates = [
        record(1, "alpha", None, false),
        record(2, "alpha", Some(user), false),
        record(3, "beta", Some(user), true),
        record(4, "gamma", None, true),
        record(5, "alpha", Some(other), false),
    ];
    let mut outputs = Vec::new();
    for mask in 0..32 {
        for reverse in [false, true] {
            for admin in [false, true] {
                let mut rows = candidates
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| mask & (1 << index) != 0)
                    .map(|(_, row)| row.clone())
                    .collect::<Vec<_>>();
                if reverse {
                    rows.reverse();
                }
                outputs.push(json!({
                    "mask": mask, "reverse": reverse, "admin": admin,
                    "entries": merge_entries(rows, user, admin)
                }));
            }
        }
    }
    compare(
        "manager",
        &json!(outputs),
        include_str!("manager-expected.json"),
    );
}

struct Fixture {
    admin: PgPool,
    pool: PgPool,
    database: String,
    role: String,
}

impl Fixture {
    async fn new() -> Self {
        let configured = std::env::var("CHART_CONTRACT_TEST_DATABASE_URL")
            .expect("explicit disposable CHART_CONTRACT_TEST_DATABASE_URL is required");
        let mut url = url::Url::parse(&configured).unwrap();
        assert_eq!(
            url.host_str(),
            Some("db"),
            "use the isolated fixture Docker network"
        );
        assert_eq!(url.path(), "/chart_fixture");
        url.set_path("/postgres");
        let admin = PgPool::connect(url.as_str()).await.unwrap();
        let suffix = Uuid::new_v4().simple().to_string();
        let database = format!("chart_contract_{suffix}");
        let role = format!("chart_role_{suffix}");
        admin
            .execute(sqlx::AssertSqlSafe(format!(
                "CREATE ROLE {role} LOGIN PASSWORD 'synthetic_chart_role_only'
             NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS"
            )))
            .await
            .unwrap();
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
             CREATE EXTENSION IF NOT EXISTS pgcrypto WITH SCHEMA public;
             CREATE EXTENSION IF NOT EXISTS timescaledb WITH SCHEMA public;
             CREATE EXTENSION IF NOT EXISTS cube WITH SCHEMA panda;
             CREATE EXTENSION IF NOT EXISTS earthdistance WITH SCHEMA panda;"
            )))
            .await
            .unwrap();
        bootstrap.close().await;
        url.set_username(&role).unwrap();
        url.set_password(Some("synthetic_chart_role_only")).unwrap();
        let migration_pool = PgPoolOptions::new()
            .max_connections(2)
            .connect(url.as_str())
            .await
            .unwrap();
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
            "database_url": url.as_str(), "redis_url": "redis://synthetic.invalid",
            "panda_env": "production"
        }))
        .unwrap();
        crate::private_deployment::history::protect_history(&pool, &config)
            .await
            .unwrap();
        let protected: bool = sqlx::query_scalar(
            "SELECT NOT rolsuper AND NOT rolbypassrls
             AND NOT has_table_privilege(current_user,'panda.trips','DELETE')
             AND NOT has_table_privilege(current_user,'timeseries.telemetry','TRUNCATE')
             FROM pg_roles WHERE rolname=current_user",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(protected);
        Self {
            admin,
            pool,
            database,
            role,
        }
    }

    async fn rows(&self) -> Vec<ChartRecord> {
        sqlx::query_as("SELECT * FROM panda.charts ORDER BY slug, owner_id NULLS FIRST")
            .fetch_all(&self.pool)
            .await
            .unwrap()
    }

    async fn insert(&self, row: ChartRecord) {
        sqlx::query(
            "INSERT INTO panda.charts
             (id,owner_id,slug,name,description,is_default,is_locked,is_enabled,config,
              baseline_revision,created_at,updated_at)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)",
        )
        .bind(row.id)
        .bind(row.owner_id)
        .bind(row.slug)
        .bind(row.name)
        .bind(row.description)
        .bind(row.is_default)
        .bind(row.is_locked)
        .bind(row.is_enabled)
        .bind(row.config)
        .bind(row.baseline_revision)
        .bind(row.created_at)
        .bind(row.updated_at)
        .execute(&self.pool)
        .await
        .unwrap();
    }

    async fn cleanup(self) {
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

fn normalized(rows: &[ChartRecord]) -> Value {
    let mut values = serde_json::to_value(rows).unwrap();
    for (row, value) in rows.iter().zip(values.as_array_mut().unwrap()) {
        if row.id.as_u128() > 15 {
            assert_eq!(
                row.created_at, row.updated_at,
                "new rows share statement timestamps"
            );
            value["id"] = json!(format!("generated:{}", row.slug));
            value["createdAt"] = json!("seed-time");
            value["updatedAt"] = json!("seed-time");
        } else if row.updated_at != "2026-01-02T00:00:00Z".parse::<DateTime<Utc>>().unwrap() {
            assert!(row.updated_at > row.created_at);
            value["updatedAt"] = json!("seed-time");
        }
    }
    values
}

#[tokio::test]
#[ignore = "requires unique disposable Docker chart fixture; run explicitly with --ignored"]
async fn complete_catalog_persistence_contract() {
    let fixture = Fixture::new().await;
    let user = Uuid::from_u128(100);
    let other = Uuid::from_u128(101);
    for (id, email) in [
        (user, "chart-one@example.invalid"),
        (other, "chart-two@example.invalid"),
    ] {
        sqlx::query("INSERT INTO panda.users(id,email,role) VALUES ($1,$2,'user')")
            .bind(id)
            .bind(email)
            .execute(&fixture.pool)
            .await
            .unwrap();
    }
    let defaults: Vec<Value> = serde_json::from_str(BUNDLED_CHART_DEFAULTS).unwrap();
    for (index, revision) in [None, Some(0), Some(4), Some(5), Some(6)]
        .into_iter()
        .enumerate()
    {
        let slug = defaults[index]["slug"].as_str().unwrap();
        let mut row = record(index as u128 + 1, slug, None, true);
        row.baseline_revision = revision;
        fixture.insert(row).await;
    }
    fixture
        .insert(record(
            10,
            defaults[0]["slug"].as_str().unwrap(),
            Some(user),
            true,
        ))
        .await;
    fixture
        .insert(record(
            11,
            defaults[0]["slug"].as_str().unwrap(),
            Some(other),
            false,
        ))
        .await;
    fixture
        .insert(record(12, "charge-session-curve", None, true))
        .await;
    fixture
        .insert(record(13, "charge-session-curve", Some(user), true))
        .await;
    fixture
        .insert(record(14, "custom-system", None, false))
        .await;
    fixture
        .insert(record(15, "custom-personal", Some(user), false))
        .await;
    let before = fixture.rows().await;
    seed_defaults(&fixture.pool).await.unwrap();
    let after = fixture.rows().await;
    seed_defaults(&fixture.pool).await.unwrap();
    let repeated = fixture.rows().await;
    assert_eq!(
        serde_json::to_value(&after).unwrap(),
        serde_json::to_value(&repeated).unwrap()
    );
    assert_eq!(after.len(), 19);
    for original in &before {
        if original.owner_id.is_some()
            || original.slug == "custom-system"
            || original.slug != "charge-session-curve"
                && original
                    .baseline_revision
                    .is_some_and(|revision| revision >= 5)
        {
            assert_eq!(
                serde_json::to_value(original).unwrap(),
                serde_json::to_value(after.iter().find(|row| row.id == original.id).unwrap())
                    .unwrap()
            );
        }
    }
    let scoped = after
        .iter()
        .filter(|row| row.owner_id.is_none() || row.owner_id == Some(user))
        .cloned()
        .collect::<Vec<_>>();
    let normalized_scoped: Vec<ChartRecord> = scoped
        .iter()
        .cloned()
        .map(|mut row| {
            if !before.iter().any(|original| original.id == row.id) {
                row.id = Uuid::from_u128(
                    1000 + defaults.iter().position(|d| d["slug"] == row.slug).unwrap() as u128,
                );
                row.created_at = "2026-01-03T00:00:00Z".parse().unwrap();
            }
            if row.updated_at != "2026-01-02T00:00:00Z".parse::<DateTime<Utc>>().unwrap() {
                row.updated_at = "2026-01-03T00:00:00Z".parse().unwrap();
            }
            row
        })
        .collect();
    let actual = json!({
        "before": normalized(&before), "after": normalized(&after), "repeated": normalized(&repeated),
        "manager": merge_entries(normalized_scoped.clone(), user, false),
        "admin": merge_entries(normalized_scoped, user, true)
    });
    if let Ok(directory) = std::env::var("CHART_CONTRACT_RUN") {
        std::fs::write(
            format!("{directory}/catalog-raw.json"),
            serde_json::to_string_pretty(&json!({
                "before": before, "after": after, "repeated": repeated
            }))
            .unwrap(),
        )
        .unwrap();
    }
    fixture.cleanup().await;
    compare("catalog", &actual, include_str!("catalog-expected.json"));
}

#[tokio::test]
#[ignore = "requires unique disposable Docker chart fixture; run explicitly with --ignored"]
async fn complete_fresh_catalog_contract() {
    let fixture = Fixture::new().await;
    assert!(fixture.rows().await.is_empty());
    seed_defaults(&fixture.pool).await.unwrap();
    let rows = fixture.rows().await;
    assert_eq!(rows.len(), 14);
    let defaults: Vec<Value> = serde_json::from_str(BUNDLED_CHART_DEFAULTS).unwrap();
    for row in &rows {
        let default = defaults
            .iter()
            .find(|item| item["slug"] == row.slug)
            .unwrap();
        assert_eq!(row.config, default["definition"]);
        assert_eq!(row.baseline_revision, Some(5));
        assert_eq!(row.owner_id, None);
        assert!(row.is_default && !row.is_locked);
    }
    seed_defaults(&fixture.pool).await.unwrap();
    assert_eq!(
        serde_json::to_value(&rows).unwrap(),
        serde_json::to_value(fixture.rows().await).unwrap()
    );
    let actual = normalized(&rows);
    fixture.cleanup().await;
    compare("fresh", &actual, include_str!("fresh-expected.json"));
}

#[tokio::test]
#[ignore = "requires unique disposable Docker chart fixture; run explicitly with --ignored"]
async fn catalog_failure_preserves_statement_progress_contract() {
    let fixture = Fixture::new().await;
    fixture
        .insert(record(1, "charge-session-curve", None, false))
        .await;
    let defaults: Vec<Value> = serde_json::from_str(BUNDLED_CHART_DEFAULTS).unwrap();
    let rejected = defaults[3]["slug"].as_str().unwrap();
    fixture.pool.execute(sqlx::AssertSqlSafe(format!(
        "ALTER TABLE panda.charts ADD CONSTRAINT synthetic_seed_failure CHECK (slug <> '{rejected}')"
    ))).await.unwrap();
    let error = seed_defaults(&fixture.pool).await.unwrap_err();
    let db_error = error
        .downcast_ref::<sqlx::Error>()
        .unwrap()
        .as_database_error()
        .unwrap();
    let rows = fixture.rows().await;
    assert_eq!(
        rows.len(),
        3,
        "retirement and earlier writes remain committed"
    );
    let actual = json!({
        "code": db_error.code(), "message": db_error.message(), "constraint": db_error.constraint(),
        "rows": normalized(&rows)
    });
    fixture.cleanup().await;
    compare("failure", &actual, include_str!("failure-expected.json"));
}
