use sqlx::migrate::Migrator;

const FIRST_NAMESPACED_UPSTREAM_VERSION: i64 = 28;
const UPSTREAM_VERSION_OFFSET: i64 = 1_000_000;

pub(super) fn compose(upstream: Migrator, private: Migrator) -> Migrator {
    assert_eq!(
        private.migrations.len(),
        1,
        "review new private schema changes explicitly"
    );
    assert_eq!(private.migrations[0].version, 28);
    assert_eq!(private.migrations[0].description, "active trip checkpoints");
    let mut migrations = upstream.migrations.into_owned();
    for migration in &mut migrations {
        assert!(
            (1..UPSTREAM_VERSION_OFFSET).contains(&migration.version),
            "upstream migration exceeds its reserved namespace"
        );
        if migration.version >= FIRST_NAMESPACED_UPSTREAM_VERSION {
            migration.version += UPSTREAM_VERSION_OFFSET;
        }
    }
    migrations.extend(private.migrations.into_owned());
    Migrator::with_migrations(migrations)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrations::{compiled_migration_ledger, validate_ledger_prefix, MIGRATOR};

    #[test]
    fn deployed_checkpoint_identity_is_preserved_and_upstream_sql_is_unchanged() {
        let upstream = sqlx::migrate!("./migrations");
        let private = sqlx::migrate!("./migrations-private");
        let existing = &private.migrations[0];
        let checkpoint = MIGRATOR
            .iter()
            .find(|migration| migration.version == 28)
            .unwrap();
        assert_eq!(checkpoint.checksum, existing.checksum);
        assert_eq!(checkpoint.description, existing.description);
        assert_eq!(checkpoint.sql.as_str(), existing.sql.as_str());
        for original in upstream.iter() {
            let version = original.version
                + if original.version >= FIRST_NAMESPACED_UPSTREAM_VERSION {
                    UPSTREAM_VERSION_OFFSET
                } else {
                    0
                };
            let mapped = MIGRATOR
                .iter()
                .find(|migration| migration.version == version)
                .unwrap();
            assert_eq!(mapped.checksum, original.checksum);
            assert_eq!(mapped.sql.as_str(), original.sql.as_str());
        }
        assert!(validate_ledger_prefix(&compiled_migration_ledger()[..28]).is_ok());
    }

    #[test]
    fn the_next_upstream_migration_appends_without_rewriting_the_existing_catalog() {
        let mut upstream = sqlx::migrate!("./migrations");
        let mut next = upstream.migrations.last().unwrap().clone();
        next.version += 1;
        upstream.migrations.to_mut().push(next);
        let future = compose(upstream, sqlx::migrate!("./migrations-private"));
        for (existing, appended) in MIGRATOR.iter().zip(future.iter()) {
            assert_eq!(existing.version, appended.version);
            assert_eq!(existing.checksum, appended.checksum);
        }
        assert_eq!(future.iter().len(), MIGRATOR.iter().len() + 1);
    }
}
