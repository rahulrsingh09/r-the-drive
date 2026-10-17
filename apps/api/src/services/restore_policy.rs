//! Table data which can never be imported from a recovery archive. Keep dump
//! redaction, TOC filtering and post-migration sanitation on one policy.
pub const PROTECTED_TABLES: &[&str] = &[
    "vehicle_credentials",
    "external_connection_activity",
    "external_connection_settings",
    "system_config",
    "authentication_settings",
    "refresh_tokens",
    "session_families",
    "api_keys",
    "account_invitation_vehicles",
    "account_invitations",
    "vehicle_invites",
    "backup_restore_requests",
    "backup_artifacts",
    "backup_runs",
    "backup_settings",
];

pub fn skip_toc_entry(line: &str) -> bool {
    let static_exclusions = [
        " EXTENSION - ",
        " COMMENT - EXTENSION ",
        " SCHEMA - public ",
        " COMMENT - SCHEMA public ",
        " SCHEMA - panda ",
        " COMMENT - SCHEMA panda ",
    ];
    static_exclusions
        .into_iter()
        .map(str::to_owned)
        .chain(
            PROTECTED_TABLES
                .iter()
                .map(|table| format!(" TABLE DATA panda {table} ")),
        )
        .any(|pattern| line.contains(&pattern))
}

pub async fn sanitize(pool: &sqlx::PgPool) -> Result<(), sqlx::Error> {
    let mut transaction = pool.begin().await?;
    let tables: Vec<String> = PROTECTED_TABLES
        .iter()
        .map(|table| format!("panda.{table}"))
        .collect();
    let present: Vec<bool> = sqlx::query_scalar(
        "SELECT to_regclass(name) IS NOT NULL FROM unnest($1::text[]) WITH ORDINALITY AS names(name, position) ORDER BY position",
    ).bind(tables).fetch_all(&mut *transaction).await?;
    for table in PROTECTED_TABLES
        .iter()
        .zip(present)
        .filter_map(|(table, exists)| exists.then_some(table))
    {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DELETE FROM panda.\"{table}\""
        )))
        .execute(&mut *transaction)
        .await?;
    }
    sqlx::query("INSERT INTO panda.authentication_settings (id) VALUES (TRUE)")
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn filters_every_protected_data_entry_without_removing_schema() {
        for table in PROTECTED_TABLES {
            assert!(skip_toc_entry(&format!(
                "52; 0 12 TABLE DATA panda {table} postgres"
            )));
            assert!(!skip_toc_entry(&format!(
                "52; 0 12 TABLE panda {table} postgres"
            )));
        }
        assert!(!skip_toc_entry(
            "52; 0 12 TABLE DATA panda vehicles postgres"
        ));
    }
}
