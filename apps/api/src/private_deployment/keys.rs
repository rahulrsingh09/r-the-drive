use age::secrecy::ExposeSecret;
use anyhow::{bail, Context};
use futures::TryStreamExt;
use sqlx::{PgPool, Postgres, Transaction};

#[cfg(test)]
#[path = "full_replacement_key_custody_keys_tests.rs"]
mod full_replacement_key_custody_tests;

#[derive(Clone)]
pub struct BootstrappedKeys {
    pub jwt_private_pem: String,
    pub jwt_public_pem: String,
    pub age_key: String,
}

impl BootstrappedKeys {
    pub fn validate(&self) -> anyhow::Result<()> {
        validate_rsa_pair(&self.jwt_private_pem, &self.jwt_public_pem)?;
        self.identity()?;
        Ok(())
    }

    fn identity(&self) -> anyhow::Result<age::x25519::Identity> {
        self.age_key.trim().parse().map_err(|_| {
            anyhow::anyhow!("AGE_ENCRYPTION_KEY must contain one valid age X25519 secret identity")
        })
    }
}

fn validate_rsa_pair(private: &str, public: &str) -> anyhow::Result<()> {
    use jsonwebtoken::{crypto, Algorithm, DecodingKey, EncodingKey};
    let signing = EncodingKey::from_rsa_pem(private.as_bytes())
        .map_err(|_| anyhow::anyhow!("JWT_SECRET must contain a valid RSA private PEM"))?;
    let verification = DecodingKey::from_rsa_pem(public.as_bytes())
        .map_err(|_| anyhow::anyhow!("JWT_PUBLIC_KEY must contain a valid RSA public PEM"))?;
    let challenge = b"panda key custody self-test";
    let signature = crypto::sign(challenge, &signing, Algorithm::RS256).map_err(|_| {
        anyhow::anyhow!("JWT_SECRET cannot sign with RS256 (RSA >= 2048 bits required)")
    })?;
    if !crypto::verify(&signature, challenge, &verification, Algorithm::RS256).unwrap_or(false) {
        bail!("JWT_SECRET and JWT_PUBLIC_KEY must be a matching RSA key pair");
    }
    Ok(())
}

pub fn external_keys(
    private: Option<&str>,
    public: Option<&str>,
    age: Option<&str>,
) -> anyhow::Result<Option<BootstrappedKeys>> {
    match (private, public, age) {
        (None, None, None) => Ok(None),
        (Some(private), Some(public), Some(age)) => {
            let keys = BootstrappedKeys {
                jwt_private_pem: private.to_owned(),
                jwt_public_pem: public.to_owned(),
                age_key: age.trim().to_owned(),
            };
            keys.validate()?;
            Ok(Some(keys))
        }
        _ => bail!("JWT_SECRET, JWT_PUBLIC_KEY, and AGE_ENCRYPTION_KEY must be supplied together (direct values or their _FILE alternatives)"),
    }
}

/// Production entry point: never generates or stores private key material.
pub async fn bootstrap_keys(
    pool: &PgPool,
    env_jwt_secret: Option<String>,
    env_jwt_public: Option<String>,
    env_age_key: Option<String>,
) -> anyhow::Result<BootstrappedKeys> {
    let keys = external_keys(env_jwt_secret.as_deref(), env_jwt_public.as_deref(), env_age_key.as_deref())?
        .context("production requires external JWT_SECRET, JWT_PUBLIC_KEY, and AGE_ENCRYPTION_KEY (or _FILE alternatives)")?;
    let mut tx = locked_transaction(pool).await?;
    if read_database_keys(&mut tx)
        .await?
        .iter()
        .any(Option::is_some)
    {
        bail!("database-backed application keys remain; stop all app instances and complete the explicit key-custody migration before external startup");
    }
    bind_age_identity(&mut tx, &keys, false).await?;
    tx.commit().await?;
    Ok(keys)
}

pub async fn bootstrap_keys_for_config(
    pool: &PgPool,
    config: &crate::config::Config,
) -> anyhow::Result<BootstrappedKeys> {
    if config.is_development()
        && config.jwt_secret.is_none()
        && config.jwt_public_key.is_none()
        && config.age_encryption_key.is_none()
    {
        bootstrap_development_keys(pool).await
    } else {
        bootstrap_keys(
            pool,
            config.jwt_secret.clone(),
            config.jwt_public_key.clone(),
            config.age_encryption_key.clone(),
        )
        .await
    }
}

/// Explicit local-development/test path; existing state is never repaired by generation.
pub async fn bootstrap_development_keys(pool: &PgPool) -> anyhow::Result<BootstrappedKeys> {
    let mut tx = locked_transaction(pool).await?;
    let stored = read_database_keys(&mut tx).await?;
    let keys = if stored.iter().all(Option::is_none) {
        if age_recipient(&mut tx).await?.is_some() || has_ciphertexts(&mut tx).await? {
            bail!("application keys are missing but encrypted state or an AGE binding exists; restore the original keys, never regenerate");
        }
        let keys = generate_keys()?;
        for (name, value) in [
            ("jwt_private_key", &keys.jwt_private_pem),
            ("jwt_public_key", &keys.jwt_public_pem),
            ("age_key", &keys.age_key),
        ] {
            sqlx::query("INSERT INTO panda.system_config (key, value) VALUES ($1, $2)")
                .bind(name)
                .bind(value)
                .execute(&mut *tx)
                .await?;
        }
        keys
    } else {
        complete_database_keys(stored)?
    };
    bind_age_identity(&mut tx, &keys, false).await?;
    tx.commit().await?;
    Ok(keys)
}

async fn locked_transaction(pool: &PgPool) -> anyhow::Result<Transaction<'_, Postgres>> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(1234567890)")
        .execute(&mut *tx)
        .await?;
    sqlx::query("LOCK TABLE panda.system_config IN SHARE ROW EXCLUSIVE MODE")
        .execute(&mut *tx)
        .await?;
    Ok(tx)
}

async fn read_database_keys(
    tx: &mut Transaction<'_, Postgres>,
) -> anyhow::Result<[Option<String>; 3]> {
    let (private, public, age): (Option<String>, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT
            (SELECT value FROM panda.system_config WHERE key = 'jwt_private_key'),
            (SELECT value FROM panda.system_config WHERE key = 'jwt_public_key'),
            (SELECT value FROM panda.system_config WHERE key = 'age_key')",
    )
    .fetch_one(&mut **tx)
    .await
    .context("reading application key custody")?;
    Ok([private, public, age])
}

fn complete_database_keys(stored: [Option<String>; 3]) -> anyhow::Result<BootstrappedKeys> {
    let [Some(private), Some(public), Some(age)] = stored else {
        bail!("database application key bundle is missing or partial; restore the originals from a protected backup, never clear rows or regenerate");
    };
    external_keys(Some(&private), Some(&public), Some(&age))?
        .context("database application key bundle is missing")
}

async fn age_recipient(tx: &mut Transaction<'_, Postgres>) -> anyhow::Result<Option<String>> {
    Ok(
        sqlx::query_scalar("SELECT value FROM panda.system_config WHERE key = 'age_recipient'")
            .fetch_optional(&mut **tx)
            .await?,
    )
}

// Extend this inventory whenever another durable AGE-encrypted field is introduced.
const CIPHERTEXT_QUERY: &str = "SELECT encrypted_tokens AS ciphertext FROM panda.vehicle_credentials
    UNION ALL SELECT secret_key_encrypted FROM panda.backup_settings WHERE secret_key_encrypted IS NOT NULL
    UNION ALL SELECT api_key_encrypted FROM panda.external_connection_settings WHERE api_key_encrypted IS NOT NULL
    UNION ALL SELECT bearer_token_encrypted FROM panda.external_connection_settings WHERE bearer_token_encrypted IS NOT NULL
    UNION ALL SELECT client_secret_encrypted FROM panda.authentication_settings WHERE client_secret_encrypted IS NOT NULL";

async fn lock_ciphertexts(tx: &mut Transaction<'_, Postgres>) -> anyhow::Result<()> {
    sqlx::query("LOCK TABLE panda.vehicle_credentials, panda.backup_settings,
        panda.external_connection_settings, panda.authentication_settings IN SHARE ROW EXCLUSIVE MODE")
        .execute(&mut **tx).await?;
    Ok(())
}

async fn has_ciphertexts(tx: &mut Transaction<'_, Postgres>) -> anyhow::Result<bool> {
    lock_ciphertexts(tx).await?;
    Ok(sqlx::query(CIPHERTEXT_QUERY)
        .fetch_optional(&mut **tx)
        .await?
        .is_some())
}

async fn bind_age_identity(
    tx: &mut Transaction<'_, Postgres>,
    keys: &BootstrappedKeys,
    verify_all: bool,
) -> anyhow::Result<()> {
    let identity = keys.identity()?;
    let recipient = identity.to_public().to_string();
    let saved = age_recipient(tx).await?;
    if saved.as_ref().is_some_and(|saved| saved != &recipient) {
        bail!("AGE_ENCRYPTION_KEY differs from the database AGE binding; restore the original key (key rotation requires a separate ciphertext migration)");
    }
    if saved.is_none() || verify_all {
        lock_ciphertexts(tx).await?;
        let mut rows = sqlx::query_scalar::<_, Vec<u8>>(CIPHERTEXT_QUERY).fetch(&mut **tx);
        while let Some(ciphertext) = rows.try_next().await? {
            let result = (|| -> anyhow::Result<()> {
                let decryptor = age::Decryptor::new(ciphertext.as_slice())?;
                let mut reader =
                    decryptor.decrypt(std::iter::once(&identity as &dyn age::Identity))?;
                std::io::copy(&mut reader, &mut std::io::sink())?;
                Ok(())
            })();
            if result.is_err() {
                bail!("existing encrypted data cannot be authenticated with AGE_ENCRYPTION_KEY; preserve the database and restore the original key/ciphertexts");
            }
        }
        drop(rows);
        sqlx::query("INSERT INTO panda.system_config (key, value) VALUES ('age_recipient', $1) ON CONFLICT (key) DO NOTHING")
            .bind(recipient).execute(&mut **tx).await?;
    }
    Ok(())
}

/// Read-only export source. The caller must save the originals outside database/backup storage.
pub async fn export_database_keys(pool: &PgPool) -> anyhow::Result<BootstrappedKeys> {
    let mut tx = locked_transaction(pool).await?;
    let keys = complete_database_keys(read_database_keys(&mut tx).await?)?;
    tx.rollback().await?;
    Ok(keys)
}

/// Explicit operator migration, after exporting and independently backing up the original bundle.
/// Preserves ciphertext bytes and removes only the three legacy key rows, atomically with the binding.
pub async fn migrate_database_keys(
    pool: &PgPool,
    external: &BootstrappedKeys,
) -> anyhow::Result<()> {
    external.validate()?;
    let mut tx = locked_transaction(pool).await?;
    let stored = read_database_keys(&mut tx).await?;
    if stored.iter().any(Option::is_some) {
        let original = complete_database_keys(stored)?;
        validate_rsa_pair(&external.jwt_private_pem, &original.jwt_public_pem)
            .context("external JWT pair does not match the original database pair")?;
        if original.identity()?.to_public() != external.identity()?.to_public() {
            bail!("external AGE key does not match the original database key; no keys or ciphertexts were changed");
        }
    }
    bind_age_identity(&mut tx, external, true).await?;
    sqlx::query("DELETE FROM panda.system_config WHERE key IN ('jwt_private_key', 'jwt_public_key', 'age_key')")
        .execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

/// Creates an owner-only bundle directory; refuses existing paths, including symlinks.
#[cfg(unix)]
pub fn write_key_bundle(
    directory: &std::path::Path,
    keys: &BootstrappedKeys,
) -> anyhow::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    keys.validate()?;
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(directory)
        .context("create a NEW key directory outside application data and backups")?;
    for (name, value) in [
        ("jwt_private.pem", &keys.jwt_private_pem),
        ("jwt_public.pem", &keys.jwt_public_pem),
        ("age_key.txt", &keys.age_key),
    ] {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(directory.join(name))
            .context("create key file without overwriting")?;
        file.write_all(value.as_bytes())?;
        file.sync_all()?;
    }
    std::fs::File::open(directory)?.sync_all()?;
    std::fs::File::open(
        directory
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(std::path::Path::new(".")),
    )?
    .sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
pub fn write_key_bundle(
    _directory: &std::path::Path,
    _keys: &BootstrappedKeys,
) -> anyhow::Result<()> {
    bail!("generate/export requires Unix owner-only file permissions; run the key utility in the Linux container");
}

pub fn read_key_bundle(directory: &std::path::Path) -> anyhow::Result<BootstrappedKeys> {
    let keys = BootstrappedKeys {
        jwt_private_pem: std::fs::read_to_string(directory.join("jwt_private.pem"))?,
        jwt_public_pem: std::fs::read_to_string(directory.join("jwt_public.pem"))?,
        age_key: std::fs::read_to_string(directory.join("age_key.txt"))?,
    };
    keys.validate()?;
    Ok(keys)
}

pub fn generate_keys() -> anyhow::Result<BootstrappedKeys> {
    use aws_lc_rs::{
        encoding::{AsDer, Pkcs8V1Der, PublicKeyX509Der},
        rsa::{KeySize, PrivateDecryptingKey},
    };

    let private_key = PrivateDecryptingKey::generate(KeySize::Rsa2048)
        .map_err(|_| anyhow::anyhow!("RSA key generation failed"))?;
    let private_der = AsDer::<Pkcs8V1Der>::as_der(&private_key)
        .map_err(|_| anyhow::anyhow!("encode private key failed"))?;
    let public_der = AsDer::<PublicKeyX509Der>::as_der(&private_key.public_key())
        .map_err(|_| anyhow::anyhow!("encode public key failed"))?;
    let jwt_private_pem = pem::encode(&pem::Pem::new("PRIVATE KEY", private_der.as_ref()));
    let jwt_public_pem = pem::encode(&pem::Pem::new("PUBLIC KEY", public_der.as_ref()));

    let age_identity = age::x25519::Identity::generate();
    let age_key = age_identity.to_string().expose_secret().to_owned();

    Ok(BootstrappedKeys {
        jwt_private_pem,
        jwt_public_pem,
        age_key,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle() -> &'static BootstrappedKeys {
        static KEYS: std::sync::OnceLock<BootstrappedKeys> = std::sync::OnceLock::new();
        KEYS.get_or_init(|| generate_keys().expect("generate test keys"))
    }

    #[test]
    fn complete_external_bundle_and_every_partial_combination() {
        let keys = bundle();
        for mask in 0..8 {
            let result = external_keys(
                (mask & 1 != 0).then_some(keys.jwt_private_pem.as_str()),
                (mask & 2 != 0).then_some(keys.jwt_public_pem.as_str()),
                (mask & 4 != 0).then_some(keys.age_key.as_str()),
            );
            match mask {
                0 => assert!(result.unwrap().is_none()),
                7 => assert!(result.unwrap().is_some()),
                _ => assert!(result.is_err(), "partial external bundle {mask} must fail"),
            }
        }
    }

    #[test]
    fn invalid_blank_and_mismatched_keys_fail_without_disclosing_material() {
        let keys = bundle();
        for invalid in ["", " ", "invalid-private-material-must-not-appear"] {
            for field in 0..3 {
                let mut candidate = keys.clone();
                match field {
                    0 => candidate.jwt_private_pem = invalid.into(),
                    1 => candidate.jwt_public_pem = invalid.into(),
                    _ => candidate.age_key = invalid.into(),
                }
                let error = candidate.validate().expect_err("invalid key rejected");
                assert!(!error
                    .to_string()
                    .contains("invalid-private-material-must-not-appear"));
            }
        }
        let mut mismatched = keys.clone();
        mismatched.jwt_public_pem = generate_keys().unwrap().jwt_public_pem;
        assert!(mismatched
            .validate()
            .unwrap_err()
            .to_string()
            .contains("matching RSA"));
    }

    #[test]
    #[cfg(unix)]
    fn bundle_files_are_private_and_never_overwritten() {
        use std::os::unix::fs::PermissionsExt;
        let directory =
            std::env::temp_dir().join(format!("custody-files-{}", uuid::Uuid::new_v4()));
        let keys = bundle();
        write_key_bundle(&directory, keys).unwrap();
        assert_eq!(
            std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
        for name in ["jwt_private.pem", "jwt_public.pem", "age_key.txt"] {
            assert_eq!(
                std::fs::metadata(directory.join(name))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        assert!(write_key_bundle(&directory, &generate_keys().unwrap()).is_err());
        let restored = read_key_bundle(&directory).unwrap();
        assert!(
            restored.age_key == keys.age_key && restored.jwt_private_pem == keys.jwt_private_pem
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    async fn external_boot(
        pool: &PgPool,
        keys: &BootstrappedKeys,
    ) -> anyhow::Result<BootstrappedKeys> {
        bootstrap_keys(
            pool,
            Some(keys.jwt_private_pem.clone()),
            Some(keys.jwt_public_pem.clone()),
            Some(keys.age_key.clone()),
        )
        .await
    }

    async fn rows(pool: &PgPool) -> Vec<(String, String)> {
        sqlx::query_as("SELECT key, value FROM panda.system_config ORDER BY key")
            .fetch_all(pool)
            .await
            .unwrap()
    }

    async fn reset(pool: &PgPool) {
        sqlx::raw_sql("TRUNCATE panda.system_config, panda.vehicle_credentials,
            panda.backup_settings, panda.external_connection_settings, panda.authentication_settings")
            .execute(pool).await.unwrap();
    }

    #[tokio::test]
    #[ignore = "requires KEY_CUSTODY_TEST_DATABASE_URL pointing to a disposable PostgreSQL server"]
    async fn database_custody_preserves_data_and_rolls_back_failed_migrations() {
        let url = std::env::var("KEY_CUSTODY_TEST_DATABASE_URL")
            .expect("explicit disposable test server required");
        let admin = PgPool::connect(&url).await.unwrap();
        let name = format!("custody_{}", uuid::Uuid::new_v4().simple());
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
            .execute(&admin)
            .await
            .unwrap();
        let mut url = url::Url::parse(&url).unwrap();
        url.set_path(&name);
        let pool = PgPool::connect(url.as_str()).await.unwrap();
        sqlx::raw_sql("CREATE SCHEMA panda;
            CREATE TABLE panda.system_config (key text PRIMARY KEY, value text NOT NULL);
            CREATE TABLE panda.vehicle_credentials (encrypted_tokens bytea NOT NULL);
            CREATE TABLE panda.backup_settings (secret_key_encrypted bytea);
            CREATE TABLE panda.external_connection_settings (api_key_encrypted bytea, bearer_token_encrypted bytea);
            CREATE TABLE panda.authentication_settings (client_secret_encrypted bytea);")
            .execute(&pool).await.unwrap();
        let keys = bundle();
        assert!(bootstrap_keys(&pool, None, None, None).await.is_err());
        assert!(
            bootstrap_keys(&pool, Some(keys.jwt_private_pem.clone()), None, None)
                .await
                .is_err()
        );
        assert!(rows(&pool).await.is_empty());

        // External startup is repeatable, pins only public identity, and rejects accidental AGE replacement.
        for _ in 0..2 {
            assert!(external_boot(&pool, keys).await.is_ok());
        }
        let original_rows = rows(&pool).await;
        assert_eq!(original_rows.len(), 1);
        assert_eq!(original_rows[0].0, "age_recipient");
        let different = generate_keys().unwrap();
        assert!(external_boot(&pool, &different).await.is_err());
        assert!(bootstrap_development_keys(&pool).await.is_err());
        assert!(rows(&pool).await == original_rows);

        // Development refuses partial bundles and missing original AGE keys without modifying any row.
        reset(&pool).await;
        sqlx::query("INSERT INTO panda.system_config VALUES ('age_key', $1)")
            .bind(&keys.age_key)
            .execute(&pool)
            .await
            .unwrap();
        let original_rows = rows(&pool).await;
        assert!(bootstrap_development_keys(&pool).await.is_err());
        assert!(rows(&pool).await == original_rows);
        reset(&pool).await;
        let ciphertext = crate::ingestion::session_store::encrypt_json(
            &"synthetic credential",
            &keys.identity().unwrap(),
        )
        .unwrap();
        sqlx::query("INSERT INTO panda.vehicle_credentials VALUES ($1)")
            .bind(&ciphertext)
            .execute(&pool)
            .await
            .unwrap();
        assert!(bootstrap_development_keys(&pool).await.is_err());
        assert!(external_boot(&pool, &different).await.is_err());
        assert!(rows(&pool).await.is_empty());
        assert!(external_boot(&pool, keys).await.is_ok());

        // A legacy DB gets one complete bundle even when development starts concurrently.
        reset(&pool).await;
        let (left, right) = tokio::join!(
            bootstrap_development_keys(&pool),
            bootstrap_development_keys(&pool)
        );
        let original = left.unwrap();
        assert!(original.age_key == right.unwrap().age_key);
        sqlx::query("DELETE FROM panda.system_config WHERE key = 'age_recipient'")
            .execute(&pool)
            .await
            .unwrap();
        let ciphertext = crate::ingestion::session_store::encrypt_json(
            &"synthetic credential",
            &original.identity().unwrap(),
        )
        .unwrap();
        for statement in [
            "INSERT INTO panda.vehicle_credentials VALUES ($1)",
            "INSERT INTO panda.backup_settings VALUES ($1)",
            "INSERT INTO panda.external_connection_settings VALUES ($1, $1)",
            "INSERT INTO panda.authentication_settings VALUES ($1)",
        ] {
            sqlx::query(statement)
                .bind(&ciphertext)
                .execute(&pool)
                .await
                .unwrap();
        }
        sqlx::query("INSERT INTO panda.system_config VALUES ('app_timezone', 'UTC')")
            .execute(&pool)
            .await
            .unwrap();
        let original_rows = rows(&pool).await;
        assert!(
            external_boot(&pool, &original).await.is_err(),
            "external startup must not silently remove DB keys"
        );
        let exported = export_database_keys(&pool).await.unwrap();
        assert!(
            exported.age_key == original.age_key
                && exported.jwt_private_pem == original.jwt_private_pem
        );
        assert!(
            rows(&pool).await == original_rows,
            "export must not mutate the DB"
        );
        assert!(migrate_database_keys(&pool, &different).await.is_err());
        let mut wrong_age = original.clone();
        wrong_age.age_key = different.age_key.clone();
        assert!(migrate_database_keys(&pool, &wrong_age).await.is_err());
        assert!(rows(&pool).await == original_rows);

        // Failure after inserting the public binding must roll back the deletion AND the binding.
        sqlx::raw_sql(
            "CREATE FUNCTION panda.fail_key_delete() RETURNS trigger LANGUAGE plpgsql AS $$
            BEGIN RAISE EXCEPTION 'synthetic migration failure'; END $$;
            CREATE TRIGGER fail_key_delete BEFORE DELETE ON panda.system_config
            FOR EACH ROW EXECUTE FUNCTION panda.fail_key_delete();",
        )
        .execute(&pool)
        .await
        .unwrap();
        assert!(migrate_database_keys(&pool, &original).await.is_err());
        assert!(
            rows(&pool).await == original_rows,
            "failed migration must preserve all original key rows"
        );
        sqlx::query("DROP TRIGGER fail_key_delete ON panda.system_config")
            .execute(&pool)
            .await
            .unwrap();

        // Corrupt ciphertext in every supported location prevents migration and leaves keys intact.
        for (table, column) in [
            ("vehicle_credentials", "encrypted_tokens"),
            ("backup_settings", "secret_key_encrypted"),
            ("external_connection_settings", "api_key_encrypted"),
            ("external_connection_settings", "bearer_token_encrypted"),
            ("authentication_settings", "client_secret_encrypted"),
        ] {
            let update = format!("UPDATE panda.{table} SET {column} = $1");
            sqlx::query(sqlx::AssertSqlSafe(update.as_str()))
                .bind(b"invalid ciphertext".as_slice())
                .execute(&pool)
                .await
                .unwrap();
            assert!(migrate_database_keys(&pool, &original).await.is_err());
            assert!(rows(&pool).await == original_rows);
            sqlx::query(sqlx::AssertSqlSafe(update.as_str()))
                .bind(&ciphertext)
                .execute(&pool)
                .await
                .unwrap();
        }
        migrate_database_keys(&pool, &original).await.unwrap();
        migrate_database_keys(&pool, &original).await.unwrap();
        for _ in 0..2 {
            external_boot(&pool, &original).await.unwrap();
        }
        let migrated_rows = rows(&pool).await;
        assert_eq!(migrated_rows.len(), 2);
        assert!(migrated_rows
            .iter()
            .any(|(key, value)| key == "app_timezone" && value == "UTC"));
        assert!(migrated_rows.iter().all(|(key, _)| ![
            "jwt_private_key",
            "jwt_public_key",
            "age_key"
        ]
        .contains(&key.as_str())));
        let ciphertexts: Vec<Vec<u8>> = sqlx::query_scalar(CIPHERTEXT_QUERY)
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(ciphertexts.len(), 5);
        for preserved in ciphertexts {
            assert!(
                preserved == ciphertext,
                "migration must preserve ciphertext bytes"
            );
            let plaintext: String = crate::ingestion::session_store::decrypt_json(
                &preserved,
                &original.identity().unwrap(),
            )
            .unwrap();
            assert!(plaintext == "synthetic credential");
        }
        pool.close().await;
        sqlx::query(sqlx::AssertSqlSafe(format!("DROP DATABASE {name}")))
            .execute(&admin)
            .await
            .unwrap();
        admin.close().await;
    }
}
