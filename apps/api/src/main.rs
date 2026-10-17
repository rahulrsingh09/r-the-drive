use std::{net::SocketAddr, sync::Arc};

use panda_api::{
    config::Config,
    db::{self, pool::create_pool},
    ingestion, keys, logging,
    middleware::auth::{AppState, JwtKeys},
    routes, services,
};
use sqlx::{postgres::PgPoolOptions, PgPool};
use tokio::{net::TcpListener, task::JoinHandle};

struct ProtectedDatabase {
    pool: PgPool,
    config: Config,
}

struct ApplicationServices {
    state: AppState,
    background: [JoinHandle<()>; 7],
}

impl ProtectedDatabase {
    async fn prepare(config: Config) -> anyhow::Result<Self> {
        if config.is_production() && config.allow_insecure_lan_http_auth {
            tracing::warn!(
                "ALLOW_INSECURE_LAN_HTTP_AUTH is enabled: browser credentials and telemetry may be intercepted on this LAN; use HTTPS behind an authenticated gateway whenever possible"
            );
        }
        let pool = create_pool(&config.database_url).await?;
        let migrator = PgPoolOptions::new()
            .max_connections(1)
            .after_connect(|connection, _| {
                Box::pin(async move {
                    sqlx::query("SET search_path = public")
                        .execute(connection)
                        .await?;
                    Ok(())
                })
            })
            .connect(&config.database_url)
            .await?;
        let migration = db::migrations::run_current_migrations(&migrator).await?;
        migrator.close().await;
        panda_api::private_deployment::history::protect_history(&pool, &config).await?;
        tracing::info!(latest_version = migration.latest_version, ledger_action = ?migration.ledger_action, "database schema is current");
        Ok(Self { pool, config })
    }

    async fn initialize(self) -> anyhow::Result<ApplicationServices> {
        let Self { pool, mut config } = self;
        let material = keys::bootstrap_keys_for_config(&pool, &config).await?;
        tracing::info!(
            cryptographic_key_source = config.cryptographic_key_source(),
            database_key_shared_fate = config.cryptographic_key_source() == "database",
            "application cryptographic keys ready"
        );
        reconcile_catalogs(&pool, &config).await;
        routes::dashboards::seed_defaults(&pool).await?;
        tracing::info!("dashboard defaults seeded");
        services::chart_registry::seed_defaults(&pool).await?;
        tracing::info!("chart defaults seeded");
        services::external_connections::ensure_defaults(&pool).await?;
        tracing::info!("external connection defaults ensured");

        let jwt_keys = Arc::new(JwtKeys::new(
            &material.jwt_private_pem,
            &material.jwt_public_pem,
        )?);
        let redis = redis::Client::open(config.redis_url.clone())?;
        services::redis_health::ping(&redis).await.map_err(|error| {
            tracing::error!(operation = "startup", error = %error, "secure_session_store.unavailable");
            anyhow::anyhow!("secure session storage is unavailable; Redis authentication failed or Redis is unreachable")
        })?;
        tracing::info!(operation = "startup", "secure_session_store.ready");
        let age_key = material.age_key;
        config.age_encryption_key = Some(age_key.clone());
        let backfill = services::charge_payload_identity::BackfillConfig::from_env()?;
        // Capture admission must be restored before the first acquisition frame.
        services::ingestion_capture::init(pool.clone()).await?;
        let supervisor =
            ingestion::start_workers(pool.clone(), redis.clone(), age_key.clone(), config.clone())
                .await?;
        let state = AppState {
            pool,
            redis,
            jwt_keys,
            age_key,
            config,
            nominatim_cache: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            supervisor,
            resources: Default::default(),
        };
        let background = [
            services::backups::start_backup_scheduler(state.pool.clone(), state.config.clone()),
            services::basemap_cache::start_maintenance(state.redis.clone()),
            services::restore_jobs::start_reconciler(state.pool.clone(), state.config.clone()),
            services::weather_enrichment::start_worker(state.pool.clone(), state.age_key.clone()),
            services::trip_enrichment::start_reconciliation_worker(state.pool.clone()),
            services::security_audit::start_retention_worker(state.pool.clone()),
            services::charge_payload_identity::start_worker(state.pool.clone(), backfill),
        ];
        Ok(ApplicationServices { state, background })
    }
}

async fn reconcile_catalogs(pool: &PgPool, config: &Config) {
    if let Err(error) = services::backups::reconcile_running_runs(pool)
        .await
        .map(|reconciled| {
            if reconciled > 0 {
                tracing::warn!(
                    reconciled,
                    "backup running jobs reconciled after API startup"
                );
            }
        })
    {
        tracing::error!(error = ?error, "backup running-job reconciliation failed");
    }
    match services::restore_jobs::reconcile_completed_jobs(pool, config).await {
        Ok(()) => tracing::info!("restore job journal reconciled"),
        Err(error) => tracing::error!(error = ?error, "restore job journal reconciliation failed"),
    }
    if let Err(error) = services::backups::reconcile_local_catalog(pool, config)
        .await
        .map(|inserted| {
            if inserted > 0 {
                tracing::info!(inserted, "local backup catalog reconciled from disk");
            }
        })
    {
        tracing::error!(error = ?error, "local backup catalog reconciliation failed");
    }
}

impl ApplicationServices {
    async fn serve(self) -> anyhow::Result<()> {
        let Self {
            state,
            background: _background,
        } = self;
        let config = state.config.clone();
        let app = routes::build_router(state);
        let address: SocketAddr =
            format!("{}:{}", config.origin_bind.panda_bind_address, config.port).parse()?;
        tracing::info!("listening on {address}");
        axum::serve(
            TcpListener::bind(address).await?,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await?;
        Ok(())
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    logging::init();
    ProtectedDatabase::prepare(Config::from_env()?)
        .await?
        .initialize()
        .await?
        .serve()
        .await
}
