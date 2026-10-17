//! Offline custody operations. Never prints key material or overwrites a bundle.
use anyhow::{bail, Context};
use panda_api::keys;
use std::path::Path;

enum Operation {
    Generate,
    Export,
    Migrate,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let [operation, directory]: [String; 2] = std::env::args()
        .skip(1)
        .collect::<Vec<_>>()
        .try_into()
        .map_err(|_| {
            anyhow::anyhow!("usage: panda-keys <generate|export-db|migrate-db> DIRECTORY")
        })?;
    let operation = match operation.as_str() {
        "generate" => Operation::Generate,
        "export-db" => Operation::Export,
        "migrate-db" => Operation::Migrate,
        _ => bail!("unknown key operation; use generate, export-db, or migrate-db"),
    };
    let directory = Path::new(&directory);
    let message = if matches!(operation, Operation::Generate) {
        let bundle = keys::generate_keys()?;
        keys::write_key_bundle(directory, &bundle)?;
        "New key bundle saved. Back it up separately before using it with a NEW database."
    } else {
        let database_url = std::env::var("DATABASE_URL")
            .context("DATABASE_URL must be supplied securely for database key operations")?;
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&database_url)
            .await
            .context("connecting for key custody operation")?;
        let result: anyhow::Result<&str> = async {
            match operation {
                Operation::Export => {
                    let bundle = keys::export_database_keys(&pool).await?;
                    keys::write_key_bundle(directory, &bundle)?;
                    Ok("Original key bundle exported; database unchanged. Back it up separately and verify it before migrate-db.")
                },
                Operation::Migrate => {
                    let bundle = keys::read_key_bundle(directory)?;
                    keys::migrate_database_keys(&pool, &bundle).await?;
                    Ok("Key custody migrated. Ciphertexts preserved; provision this same bundle externally before starting the app.")
                },
                Operation::Generate => unreachable!("generation does not open a database"),
            }
        }.await;
        pool.close().await;
        result?
    };
    println!("{message}");
    Ok(())
}
