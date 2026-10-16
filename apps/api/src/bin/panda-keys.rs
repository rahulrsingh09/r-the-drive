//! Offline custody operations. Never prints key material or overwrites a bundle.
use anyhow::{bail, Context};
use panda_api::keys;
use std::path::Path;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [operation, directory] = args.as_slice() else {
        bail!("usage: panda-keys <generate|export-db|migrate-db> DIRECTORY");
    };
    let directory = Path::new(directory);
    match operation.as_str() {
        "generate" => {
            keys::write_key_bundle(directory, &keys::generate_keys()?)?;
            println!(
                "New key bundle saved. Back it up separately before using it with a NEW database."
            );
        }
        "export-db" | "migrate-db" => {
            let database_url = std::env::var("DATABASE_URL")
                .context("DATABASE_URL must be supplied securely for database key operations")?;
            let pool = sqlx::postgres::PgPoolOptions::new()
                .max_connections(1)
                .connect(&database_url)
                .await
                .context("connecting for key custody operation")?;
            if operation == "export-db" {
                keys::write_key_bundle(directory, &keys::export_database_keys(&pool).await?)?;
                println!("Original key bundle exported; database unchanged. Back it up separately and verify it before migrate-db.");
            } else {
                keys::migrate_database_keys(&pool, &keys::read_key_bundle(directory)?).await?;
                println!("Key custody migrated. Ciphertexts preserved; provision this same bundle externally before starting the app.");
            }
            pool.close().await;
        }
        _ => bail!("unknown key operation; use generate, export-db, or migrate-db"),
    }
    Ok(())
}
