//! Read-only charge-session diagnostics for canonicalization work.
//!
//! Usage:
//!   cargo run --bin diagnose_charge_sessions -- [--vehicle <uuid>]

use anyhow::{Context, Result};
use panda_api::services::charge_sessions::diagnose_charge_sessions;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

#[derive(Debug, Clone)]
struct Args {
    vehicle_id: Option<Uuid>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let args = parse_args()?;
    let database_url = std::env::var("DATABASE_URL").context("DATABASE_URL must be set")?;
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await?;
    let outcome = async {
        let diagnostics = diagnose_charge_sessions(&pool, args.vehicle_id).await?;
        println!("{}", serde_json::to_string_pretty(&diagnostics)?);
        Ok(())
    }
    .await;
    pool.close().await;
    outcome
}

fn parse_args() -> Result<Args> {
    use panda_api::db::maintenance::{command_options, OptionKind};
    let options = command_options(
        &[OptionKind::Vehicle],
        "Usage: cargo run --bin diagnose_charge_sessions -- [--vehicle <uuid>]",
        "unknown argument",
    )?;
    Ok(Args {
        vehicle_id: options.vehicle_id,
    })
}
