use super::*;

#[derive(Clone, Copy)]
pub(super) enum LedgerAdoption {
    Prefix(usize),
    BaselineProof,
}

impl LedgerAdoption {
    fn for_source(source: &DatabaseProfile) -> Self {
        if migrations::validate_ledger_prefix(&source.migration_ledger).is_ok() {
            Self::Prefix(source.migration_ledger.len())
        } else {
            Self::BaselineProof
        }
    }

    fn pending(self) -> Vec<i64> {
        let adopted = match self {
            Self::Prefix(count) => count,
            Self::BaselineProof => 1,
        };
        compiled_migration_ledger()
            .into_iter()
            .skip(adopted)
            .map(|migration| migration.version)
            .collect()
    }
}

pub(super) struct PlanInput {
    pub(super) format: String,
    pub(super) source: DatabaseProfile,
    pub(super) target: DatabaseProfile,
}

impl PlanInput {
    fn blocking_errors(&self, dump: Option<&DumpInspection>) -> Vec<RestoreBlockingError> {
        let mut errors = Vec::new();
        let source = &self.source;
        let target = &self.target;
        let mut block = |code, message| errors.push(RestoreBlockingError { code, message });
        match self.format.as_str() {
            RECOVERY_FORMAT_V3 => {
                if let Err(error) = validate_source_schema_metadata(source) {
                    block(error.code, error.message);
                }
            }
            RECOVERY_FORMAT_V1 | RECOVERY_FORMAT_V2 => block(
                RestoreBlockingCode::UnsupportedMigrationChain,
                "This package predates the public migration baseline and is retained for rollback only. Create a new v3 package after adopting the baseline.".into(),
            ),
            _ => block(RestoreBlockingCode::UnsupportedPackageFormat,
                format!("Unsupported recovery package format: {}", self.format)),
        }
        if dump.is_some_and(|dump| !dump.has_panda_schema || !dump.has_timeseries_schema) {
            block(
                RestoreBlockingCode::SchemaContractMismatch,
                "The recovery dump is missing required Panda schemas.".into(),
            );
        }
        if !target.migration_ledger_successful {
            block(
                RestoreBlockingCode::TargetMigrationDrift,
                "The target migration ledger contains a failed migration.".into(),
            );
        }
        if let Err(error) = migrations::validate_complete_ledger(&target.migration_ledger) {
            let error = blocking_from_ledger_error(true, error);
            block(error.code, error.message);
        }
        if matches!(
            LedgerAdoption::for_source(source),
            LedgerAdoption::Prefix(_)
        ) && source.migration_version > latest_migration_version()
        {
            block(
                RestoreBlockingCode::NewerSourceSchema,
                format!(
                    "The package schema version {} is newer than this release version {}.",
                    source.migration_version,
                    latest_migration_version(),
                ),
            );
        }
        if let (Some(source_major), Some(target_major)) =
            (source.postgres_major, target.postgres_major)
        {
            if source_major > target_major {
                block(RestoreBlockingCode::UnsupportedPostgresVersion, format!(
                    "PostgreSQL {source_major} packages cannot be restored into PostgreSQL {target_major}."
                ));
            }
        }
        if !timescale_versions_compatible(
            source.timescale_version.as_deref(),
            target.timescale_version.as_deref(),
        ) {
            block(
                RestoreBlockingCode::UnsupportedTimescaleVersion,
                format!(
                    "TimescaleDB source {:?} is not compatible with target {:?}.",
                    source.timescale_version, target.timescale_version,
                ),
            );
        }
        // Preserve the serialized v3 error order used by existing clients.
        if self.format == RECOVERY_FORMAT_V3 {
            if let Err(error) = validate_source_schema_metadata(source) {
                block(error.code, error.message);
            }
        }
        errors
    }

    pub(super) fn build(
        self,
        checksum: &str,
        dump: Option<&DumpInspection>,
    ) -> Result<RestorePlan, AppError> {
        let blocking_errors = self.blocking_errors(dump);
        let pending_migrations = LedgerAdoption::for_source(&self.source).pending();
        let transforms: Vec<RestoreTransform> = Vec::new();
        let validation_checks: Vec<String> = [
            "v3_schema_contract",
            "candidate_schema_matches_declared_fingerprint",
            "legacy_ledger_requires_public_baseline_contract",
            "schema_contract",
            "timescale_objects",
            "foreign_key_integrity",
            "application_health",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        let material = serde_json::to_vec(&serde_json::json!({
            "engine_version": RESTORE_ENGINE_VERSION,
            "package_checksum_sha256": checksum, "package_format": self.format,
            "source": self.source, "target": self.target,
            "compiled_catalog": compiled_migration_ledger(),
            "compiled_catalog_digest": migrations::migration_catalog_digest(),
            "schema_contract_version": SCHEMA_CONTRACT_VERSION,
            "dump": dump, "pending_migrations": pending_migrations,
            "transforms": transforms, "validation_checks": validation_checks,
        }))
        .map_err(|error| AppError::Internal(anyhow::anyhow!(error)))?;
        Ok(RestorePlan {
            plan_id: hex::encode(Sha256::digest(material)),
            engine_version: RESTORE_ENGINE_VERSION,
            package_checksum_sha256: checksum.into(),
            package_format: self.format,
            compatible: blocking_errors.is_empty(),
            source: self.source,
            target: self.target,
            pending_migrations,
            transforms,
            validation_checks,
            warnings: Vec::new(),
            blocking_errors,
            planned_at: Utc::now(),
        })
    }
}

pub(super) struct CandidateSource {
    profile: DatabaseProfile,
    adoption: LedgerAdoption,
}

pub(super) struct VerifiedCandidate {
    source: CandidateSource,
    report: SchemaContractReport,
}

pub(super) struct MigratedCandidate {
    source_report: SchemaContractReport,
    migrations: Vec<i64>,
}

impl CandidateSource {
    pub(super) fn admit(manifest: &Value) -> anyhow::Result<Self> {
        if manifest
            .get("format")
            .and_then(Value::as_str)
            .unwrap_or_default()
            != RECOVERY_FORMAT_V3
        {
            anyhow::bail!("recovery package belongs to an unsupported migration chain");
        }
        let profile = source_profile_from_manifest(manifest)?;
        validate_source_schema_metadata(&profile)
            .map_err(|error| anyhow::anyhow!(error.message))?;
        let adoption = LedgerAdoption::for_source(&profile);
        Ok(Self { profile, adoption })
    }

    pub(super) async fn verify(self, pool: &PgPool) -> anyhow::Result<VerifiedCandidate> {
        let charts_required = matches!(self.adoption, LedgerAdoption::Prefix(_))
            && self.profile.migration_version >= 11;
        let report = validate_schema_contract_for_source(pool, charts_required).await?;
        if !complete(&report) {
            anyhow::bail!(
                "restored candidate does not match a complete Panda schema contract: missing={:?}, telemetry_hypertable={}, foreign_keys_validated={}",
                report.missing_relations, report.telemetry_hypertable, report.foreign_keys_validated,
            );
        }
        let expected = self.profile.schema_fingerprint.as_deref().ok_or_else(|| {
            anyhow::anyhow!("v3 recovery package is missing its schema fingerprint")
        })?;
        if self.profile.schema_contract_version.as_deref() != Some(SCHEMA_CONTRACT_VERSION) {
            anyhow::bail!("recovery package uses an unsupported schema contract version");
        }
        if expected != report.schema_fingerprint {
            anyhow::bail!(
                "recovery package schema contract mismatch: declared {expected}, restored {}",
                report.schema_fingerprint,
            );
        }
        Ok(VerifiedCandidate {
            source: self,
            report,
        })
    }
}

impl VerifiedCandidate {
    pub(super) async fn migrate(
        self,
        pool: &PgPool,
        administrative_url: &str,
    ) -> anyhow::Result<MigratedCandidate> {
        match self.source.adoption {
            LedgerAdoption::Prefix(_) => {
                migrations::restore_ledger(pool, &self.source.profile.migration_ledger).await?
            }
            LedgerAdoption::BaselineProof => {
                if self.report.schema_fingerprint
                    != baseline_schema_fingerprint(administrative_url).await?
                {
                    anyhow::bail!("recovery package uses an unrecognized migration ledger and its restored schema is not the public baseline");
                }
                migrations::restore_baseline_ledger(pool).await?;
            }
        }
        let mut connection = pool.acquire().await?;
        sqlx::query("SET search_path = public")
            .execute(&mut *connection)
            .await?;
        inject_candidate_fault("target_migrations")?;
        MIGRATOR.run_direct(None, &mut *connection, false).await?;
        drop(connection);
        crate::services::restore_policy::sanitize(pool).await?;
        let mut connection = pool.acquire().await?;
        ensure_sanitized_authentication_settings(&mut *connection).await?;
        drop(connection);
        Ok(MigratedCandidate {
            source_report: self.report,
            migrations: self.source.adoption.pending(),
        })
    }
}

impl MigratedCandidate {
    pub(super) async fn verify(self, pool: &PgPool) -> anyhow::Result<CandidatePreparationReport> {
        let profile = runtime_database_profile(pool).await?;
        migrations::validate_complete_ledger(&profile.migration_ledger)
            .map_err(|error| anyhow::anyhow!("candidate migration ledger invalid: {error}"))?;
        if !profile.migration_ledger_successful {
            anyhow::bail!("candidate migration ledger contains a failed migration");
        }
        let target_schema = validate_schema_contract(pool).await?;
        if !complete(&target_schema) {
            anyhow::bail!("candidate final schema contract is incomplete");
        }
        Ok(CandidatePreparationReport {
            foreign_keys_validated: target_schema.foreign_keys_validated,
            source_schema: self.source_report,
            target_schema,
            applied_transforms: Vec::new(),
            migrations_applied: self.migrations,
        })
    }
}

fn complete(report: &SchemaContractReport) -> bool {
    report.required_relations_present
        && report.telemetry_hypertable
        && report.foreign_keys_validated
}
