use super::create_pool;
use crate::{db::migrations::run_current_migrations, models::geofence::match_geofence};
use anyhow::Context;
use sqlx::{postgres::PgPoolOptions, Executor};
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires an explicitly configured disposable PostgreSQL/TimescaleDB administrator"]
async fn migrated_application_pool_resolves_geospatial_functions() {
    exercise_pool(false).await;
}

#[tokio::test]
#[ignore = "requires an explicitly configured disposable PostgreSQL/TimescaleDB administrator"]
async fn application_pool_resolves_string_body_extension_dependencies() {
    exercise_pool(true).await;
}

async fn exercise_pool(legacy_function_bodies: bool) {
    let base_url = std::env::var("DATABASE_URL").expect("set disposable DATABASE_URL explicitly");
    let (prefix, _) = base_url.rsplit_once('/').expect("database URL");
    let database_name = format!("panda_geospatial_test_{}", Uuid::new_v4().simple());
    let reader_role = format!("{database_name}_reader");
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&format!("{prefix}/postgres"))
        .await
        .expect("connect to disposable database administrator");
    admin
        .execute(sqlx::AssertSqlSafe(format!(
            "CREATE DATABASE \"{database_name}\""
        )))
        .await
        .expect("create unique disposable database");
    admin
        .execute(sqlx::AssertSqlSafe(format!(
            "CREATE ROLE \"{reader_role}\" LOGIN PASSWORD 'geo_synthetic_only'
             NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS"
        )))
        .await
        .expect("create restricted disposable runtime role");
    let database_url = format!("{prefix}/{database_name}");

    // Match startup: runtime connections may already exist before the separate
    // migration pool creates schemas and resets its connection to public.
    let pool = create_pool(&database_url)
        .await
        .expect("normal runtime pool");
    let result: anyhow::Result<()> = async {
        let mut existing = pool.acquire().await?;
        let migration_pool = PgPoolOptions::new()
            .max_connections(1)
            .after_connect(|connection, _| {
                Box::pin(async move {
                    sqlx::query("SET search_path = public")
                        .execute(connection)
                        .await?;
                    Ok(())
                })
            })
            .connect(&database_url)
            .await?;
        let migrated = run_current_migrations(&migration_pool).await;
        migration_pool.close().await;
        migrated?;
        if legacy_function_bodies {
            // Test-only representative 1.1 bodies after real supported migrations.
            // Installing 1.1 before the fresh baseline also affects migration-time
            // index parsing, which is separate from runtime pool configuration.
            sqlx::raw_sql(
                "CREATE OR REPLACE FUNCTION panda.ll_to_earth(float8,float8)
                 RETURNS panda.earth LANGUAGE SQL IMMUTABLE STRICT PARALLEL SAFE
                 AS 'SELECT cube(cube(cube(earth()*cos(radians($1))*cos(radians($2))),
                     earth()*cos(radians($1))*sin(radians($2))),earth()*sin(radians($1)))::earth';
                 CREATE OR REPLACE FUNCTION panda.earth_distance(panda.earth,panda.earth)
                 RETURNS float8 LANGUAGE SQL IMMUTABLE STRICT PARALLEL SAFE
                 AS 'SELECT sec_to_gc(cube_distance($1,$2))';
                 CREATE OR REPLACE FUNCTION panda.earth_box(panda.earth,float8)
                 RETURNS panda.cube LANGUAGE SQL IMMUTABLE STRICT PARALLEL SAFE
                 AS 'SELECT cube_enlarge($1,gc_to_sec($2),3)';",
            )
            .execute(&pool)
            .await?;
        }

        let path: String = sqlx::query_scalar("SHOW search_path")
            .fetch_one(&mut *existing)
            .await?;
        eprintln!("normal application search_path: {path}");
        let extension_schemas: Vec<(String, String)> = sqlx::query_as(
            "SELECT e.extname::text, n.nspname::text
             FROM pg_catalog.pg_extension e
             JOIN pg_catalog.pg_namespace n ON n.oid=e.extnamespace
             WHERE e.extname IN ('cube','earthdistance') ORDER BY e.extname",
        )
        .fetch_all(&mut *existing)
        .await?;
        assert_eq!(
            extension_schemas,
            vec![
                ("cube".into(), "panda".into()),
                ("earthdistance".into(), "panda".into())
            ]
        );
        let schema_create_grants: Vec<(String, bool)> = sqlx::query_as(
            "SELECT nspname::text, EXISTS (
               SELECT 1 FROM pg_catalog.aclexplode(
                 COALESCE(nspacl, pg_catalog.acldefault('n',nspowner))) acl
               WHERE acl.grantee=0 AND acl.privilege_type='CREATE')
             FROM pg_catalog.pg_namespace WHERE nspname IN ('panda','public')
             ORDER BY nspname",
        )
        .fetch_all(&mut *existing)
        .await?;
        eprintln!("PUBLIC schema CREATE grants: {schema_create_grants:?}");
        assert_eq!(
            schema_create_grants,
            vec![("panda".into(), false), ("public".into(), false)]
        );

        // String bodies resolve nested lookups at execution; parsed SQL bodies
        // bind dependencies at creation. Neither needs caller path overrides.
        let qualified = sqlx::query_scalar::<_, f64>(
            "SELECT panda.earth_distance(panda.ll_to_earth(0, 0), panda.ll_to_earth(0, 1))",
        )
        .fetch_one(&mut *existing)
        .await;
        eprintln!("qualified geospatial result: {qualified:?}");
        let normal = sqlx::query_scalar::<_, f64>(
            "SELECT earth_distance(ll_to_earth(0, 0), ll_to_earth(0, 1))",
        )
        .fetch_one(&mut *existing)
        .await;
        eprintln!("ordinary geospatial result: {normal:?}");
        let distance = normal.context("normal migrated application geospatial query")?;
        assert!((111_000.0..112_000.0).contains(&distance));
        assert_eq!(qualified?, distance);

        // Holding the first connection forces a newly established connection.
        let mut later = pool.acquire().await?;
        let later_distance: f64 =
            sqlx::query_scalar("SELECT earth_distance(ll_to_earth(0, 0), ll_to_earth(0, 1))")
                .fetch_one(&mut *later)
                .await?;
        assert_eq!(later_distance, distance);
        drop(later);

        // Do not let public or temporary objects shadow extension functions or
        // types. public remains available for installed TimescaleDB helpers.
        sqlx::raw_sql(
            "CREATE FUNCTION public.earth() RETURNS double precision
             LANGUAGE SQL IMMUTABLE AS 'SELECT -1::float8';
             CREATE TEMP TABLE earth(value integer);
             CREATE TEMP TABLE cube(value integer);",
        )
        .execute(&mut *existing)
        .await?;
        let unshadowed: (f64, bool, bool, bool) = sqlx::query_as(
            "SELECT earth_distance(ll_to_earth(0, 0), ll_to_earth(0, 1)),
               earth_box(ll_to_earth(0, 0), 1000) @> ll_to_earth(0, 0),
               earth() = panda.earth(),
               'earth'::regtype = 'panda.earth'::regtype
                 AND 'cube'::regtype = 'panda.cube'::regtype",
        )
        .fetch_one(&mut *existing)
        .await?;
        assert_eq!(unshadowed, (distance, true, true, true));
        let bucket: String = sqlx::query_scalar(
            "SELECT time_bucket('1 hour'::interval,
               '2026-01-01 12:34:00+00'::timestamptz)::text",
        )
        .fetch_one(&mut *existing)
        .await?;
        assert!(bucket.contains("12:00:00"));

        let user = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO panda.users(id,email,password_hash) VALUES($1,$2,'unused-fixture')",
        )
        .bind(user)
        .bind(format!("{user}@example.invalid"))
        .execute(&pool)
        .await?;
        sqlx::query(
            "INSERT INTO panda.geofences(user_id,name,latitude,longitude,radius_m)
             VALUES($1,'Synthetic origin',0,0,1000)",
        )
        .bind(user)
        .execute(&pool)
        .await?;
        let matched = match_geofence(&pool, user, 0.0, 0.0)
            .await?
            .context("geofence match at its center")?;
        assert_eq!(matched.name, "Synthetic origin");
        assert_eq!(matched.distance_m, 0.0);
        assert!(match_geofence(&pool, user, 0.0, 1.0).await?.is_none());

        // The lookup needs no elevated role attributes or history-write grant.
        pool.execute(sqlx::AssertSqlSafe(format!(
            "GRANT USAGE ON SCHEMA panda TO \"{reader_role}\";
             GRANT SELECT ON panda.geofences TO \"{reader_role}\""
        )))
        .await?;
        let mut reader_url = url::Url::parse(&database_url)?;
        reader_url.set_username(&reader_role).expect("fixture role");
        reader_url
            .set_password(Some("geo_synthetic_only"))
            .expect("fixture password");
        let restricted = create_pool(reader_url.as_str()).await?;
        let restricted_result: anyhow::Result<()> = async {
            let elevated: bool = sqlx::query_scalar(
                "SELECT rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls
                 FROM pg_catalog.pg_roles WHERE rolname=current_user",
            )
            .fetch_one(&restricted)
            .await?;
            assert!(!elevated);
            let history_write: bool = sqlx::query_scalar(
                "SELECT has_table_privilege(current_user,'panda.trips','DELETE')
                 OR has_table_privilege(current_user,'panda.trips','TRUNCATE')",
            )
            .fetch_one(&restricted)
            .await?;
            assert!(!history_write);
            assert!(match_geofence(&restricted, user, 0.0, 0.0).await?.is_some());
            Ok(())
        }
        .await;
        restricted.close().await;
        restricted_result?;
        Ok(())
    }
    .await;
    pool.close().await;
    admin
        .execute(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE \"{database_name}\" WITH (FORCE)"
        )))
        .await
        .expect("drop disposable database");
    admin
        .execute(sqlx::AssertSqlSafe(format!("DROP ROLE \"{reader_role}\"")))
        .await
        .expect("drop restricted disposable runtime role");
    admin.close().await;
    result.expect("geospatial functions must work through the normal application pool");
}
