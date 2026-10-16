use super::{
    clear_expired_payload_aliases, find_exact_payload_survivor, repair_payload_aliases,
    ChargePayloadRecordOutcome, ChargeSessionPayloadRef,
};
use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use uuid::Uuid;

type Tx<'a> = Transaction<'a, Postgres>;

struct Incoming<'a> {
    vehicle: Uuid,
    operation: &'a str,
    transaction: Option<&'a str>,
    rivian_vehicle: Option<&'a str>,
    payload: Value,
    key: Vec<u8>,
    fingerprint: Vec<u8>,
}

#[derive(FromRow)]
struct Identity {
    vehicle_id: Uuid,
    operation: String,
    payload_fingerprint: Vec<u8>,
    canonical_payload_id: Option<Uuid>,
}

#[derive(FromRow)]
struct Canonical {
    captured_at: DateTime<Utc>,
    charge_session_id: Option<Uuid>,
    vehicle_id: Uuid,
    operation: String,
    rivian_transaction_id: Option<String>,
    rivian_vehicle_id: Option<String>,
    payload_fingerprint: Option<Vec<u8>>,
    semantic_match: bool,
}

#[derive(Clone, Copy)]
struct Candidate {
    id: Uuid,
    captured_at: DateTime<Utc>,
    session: Option<Uuid>,
}

impl Candidate {
    fn outcome(self, unchanged: bool, repaired: bool) -> ChargePayloadRecordOutcome {
        ChargePayloadRecordOutcome::Recorded {
            payload_ref: ChargeSessionPayloadRef {
                payload_id: self.id,
                captured_at: self.captured_at,
                charge_session_id: self.session,
                unchanged,
            },
            repaired,
        }
    }
}

// The plan retains the distinction between expired evidence and an absent identity.
enum Resolution {
    Reuse {
        candidate: Candidate,
        fingerprint_missing: bool,
    },
    Repair {
        candidate: Candidate,
        previous: Option<Uuid>,
    },
    Expire {
        previous: Option<Uuid>,
    },
    ClaimHistorical(Candidate),
    Create,
}

pub(super) async fn record(
    pool: &PgPool,
    vehicle_id: Uuid,
    operation: &str,
    rivian_transaction_id: Option<&str>,
    rivian_vehicle_id: Option<&str>,
    payload: Value,
) -> Result<ChargePayloadRecordOutcome> {
    let mut tx = pool.begin().await?;
    let (key, fingerprint) = sqlx::query_as::<_, (Vec<u8>, Vec<u8>)>(
        "SELECT panda.charge_payload_identity_key($1::uuid,$2,$3,$4,panda.charge_payload_fingerprint($5::jsonb)), panda.charge_payload_fingerprint($5::jsonb)",
    )
    .bind(vehicle_id).bind(operation).bind(rivian_transaction_id)
    .bind(rivian_vehicle_id).bind(&payload).fetch_one(&mut *tx).await?;
    let incoming = Incoming {
        vehicle: vehicle_id,
        operation,
        transaction: rivian_transaction_id,
        rivian_vehicle: rivian_vehicle_id,
        payload,
        key,
        fingerprint,
    };
    let plan = incoming.resolve(&mut tx).await?;
    let outcome = incoming.apply(&mut tx, plan).await?;
    tx.commit().await?;
    Ok(outcome)
}

impl Incoming<'_> {
    async fn survivor(&self, tx: &mut Tx<'_>) -> Result<Option<Candidate>> {
        Ok(find_exact_payload_survivor(
            tx,
            self.vehicle,
            self.operation,
            self.transaction,
            self.rivian_vehicle,
            &self.fingerprint,
        )
        .await?
        .map(|(id, captured_at, session)| Candidate {
            id,
            captured_at,
            session,
        }))
    }

    async fn resolve(&self, tx: &mut Tx<'_>) -> Result<Resolution> {
        let identity = sqlx::query_as::<_, Identity>(
            "SELECT vehicle_id, operation, payload_fingerprint, canonical_payload_id
             FROM panda.rivian_charge_payload_identities WHERE identity_key=$1 FOR UPDATE",
        )
        .bind(&self.key)
        .fetch_optional(&mut **tx)
        .await?;
        let Some(identity) = identity else {
            return Ok(match self.survivor(tx).await? {
                Some(candidate) => Resolution::ClaimHistorical(candidate),
                None => Resolution::Create,
            });
        };
        if identity.vehicle_id != self.vehicle
            || identity.operation != self.operation
            || identity.payload_fingerprint != self.fingerprint
        {
            return Err(anyhow!("charge payload identity metadata is inconsistent"));
        }
        let previous = identity.canonical_payload_id;
        let canonical = if let Some(id) = previous {
            sqlx::query_as::<_, Canonical>(
                "SELECT payload.captured_at,
                    CASE WHEN cs.vehicle_id=payload.vehicle_id THEN payload.charge_session_id ELSE NULL END AS charge_session_id,
                    payload.vehicle_id,payload.operation,payload.rivian_transaction_id,
                    payload.rivian_vehicle_id,payload.payload_fingerprint,
                    panda.charge_payload_fingerprint(payload.payload)=$2 AS semantic_match
                 FROM panda.rivian_charge_payloads payload
                 LEFT JOIN panda.charge_sessions cs ON cs.id=payload.charge_session_id
                 WHERE payload.id=$1",
            ).bind(id).bind(&self.fingerprint).fetch_optional(&mut **tx).await?
        } else {
            None
        };
        if let (Some(id), Some(row)) = (previous, canonical.as_ref()) {
            if row.vehicle_id == self.vehicle
                && row.operation == self.operation
                && row.rivian_transaction_id.as_deref() == self.transaction
                && row.rivian_vehicle_id.as_deref() == self.rivian_vehicle
                && row.semantic_match
            {
                return Ok(Resolution::Reuse {
                    candidate: Candidate {
                        id,
                        captured_at: row.captured_at,
                        session: row.charge_session_id,
                    },
                    fingerprint_missing: row.payload_fingerprint.as_deref()
                        != Some(self.fingerprint.as_slice()),
                });
            }
        }
        if let Some(candidate) = self.survivor(tx).await? {
            return Ok(Resolution::Repair {
                candidate,
                previous,
            });
        }
        if canonical.is_some() {
            return Err(anyhow!("charge payload canonical evidence is inconsistent"));
        }
        Ok(Resolution::Expire { previous })
    }

    async fn apply(&self, tx: &mut Tx<'_>, plan: Resolution) -> Result<ChargePayloadRecordOutcome> {
        match plan {
            Resolution::Reuse {
                candidate,
                fingerprint_missing,
            } => {
                if fingerprint_missing {
                    self.fill_fingerprint(tx, candidate.id).await?;
                }
                Ok(candidate.outcome(true, false))
            }
            Resolution::Repair {
                candidate,
                previous,
            } => {
                self.fill_fingerprint(tx, candidate.id).await?;
                self.point_identity(tx, Some(candidate.id)).await?;
                if let Some(previous) = previous {
                    repair_payload_aliases(tx, previous, candidate.id, candidate.captured_at)
                        .await?;
                }
                Ok(candidate.outcome(true, true))
            }
            Resolution::Expire { previous } => self.expire(tx, previous).await,
            Resolution::ClaimHistorical(candidate) => self.claim(tx, candidate, false).await,
            Resolution::Create => {
                let candidate = self.insert_candidate(tx).await?;
                self.claim(tx, candidate, true).await
            }
        }
    }

    async fn fill_fingerprint(&self, tx: &mut Tx<'_>, id: Uuid) -> Result<()> {
        sqlx::query("UPDATE panda.rivian_charge_payloads SET payload_fingerprint=$2 WHERE id=$1")
            .bind(id)
            .bind(&self.fingerprint)
            .execute(&mut **tx)
            .await?;
        Ok(())
    }

    async fn point_identity(&self, tx: &mut Tx<'_>, id: Option<Uuid>) -> Result<()> {
        sqlx::query("UPDATE panda.rivian_charge_payload_identities SET canonical_payload_id=$2 WHERE identity_key=$1")
            .bind(&self.key).bind(id).execute(&mut **tx).await?;
        Ok(())
    }

    async fn expire(
        &self,
        tx: &mut Tx<'_>,
        previous: Option<Uuid>,
    ) -> Result<ChargePayloadRecordOutcome> {
        clear_expired_payload_aliases(tx, previous).await?;
        self.point_identity(tx, None).await?;
        Ok(ChargePayloadRecordOutcome::EvidenceExpired)
    }

    async fn insert_candidate(&self, tx: &mut Tx<'_>) -> Result<Candidate> {
        let session = if let Some(transaction) = self.transaction {
            sqlx::query_scalar("SELECT cs.id FROM panda.charge_sessions cs LEFT JOIN panda.charge_session_external_aliases alias ON alias.charge_session_id=cs.id WHERE cs.vehicle_id=$1 AND (cs.rivian_session_id=$2 OR alias.external_id=$2) ORDER BY cs.started_at DESC LIMIT 1")
                .bind(self.vehicle).bind(transaction).fetch_optional(&mut **tx).await?
        } else {
            None
        };
        let id = Uuid::new_v4();
        let captured_at = sqlx::query_scalar("INSERT INTO panda.rivian_charge_payloads(id,vehicle_id,charge_session_id,operation,rivian_transaction_id,rivian_vehicle_id,payload_fingerprint,payload) VALUES($1,$2,$3,$4,$5,$6,$7,$8) RETURNING captured_at")
            .bind(id).bind(self.vehicle).bind(session).bind(self.operation).bind(self.transaction)
            .bind(self.rivian_vehicle).bind(&self.fingerprint).bind(&self.payload)
            .fetch_one(&mut **tx).await?;
        Ok(Candidate {
            id,
            captured_at,
            session,
        })
    }

    async fn claim(
        &self,
        tx: &mut Tx<'_>,
        candidate: Candidate,
        inserted: bool,
    ) -> Result<ChargePayloadRecordOutcome> {
        let won = sqlx::query_scalar::<_, Vec<u8>>(
            "INSERT INTO panda.rivian_charge_payload_identities(identity_key,vehicle_id,operation,payload_fingerprint,canonical_payload_id) VALUES($1,$2,$3,$4,$5) ON CONFLICT(identity_key) DO NOTHING RETURNING identity_key",
        ).bind(&self.key).bind(self.vehicle).bind(self.operation).bind(&self.fingerprint)
            .bind(candidate.id).fetch_optional(&mut **tx).await?.is_some();
        if won {
            return Ok(candidate.outcome(!inserted, false));
        }
        if inserted {
            sqlx::query("DELETE FROM panda.rivian_charge_payloads WHERE id=$1")
                .bind(candidate.id)
                .execute(&mut **tx)
                .await?;
        }
        let previous: Option<Uuid> = sqlx::query_scalar(
            "SELECT canonical_payload_id FROM panda.rivian_charge_payload_identities WHERE identity_key=$1 FOR UPDATE",
        ).bind(&self.key).fetch_one(&mut **tx).await?;
        if !inserted {
            self.point_identity(tx, Some(candidate.id)).await?;
            if let Some(previous) = previous.filter(|id| *id != candidate.id) {
                repair_payload_aliases(tx, previous, candidate.id, candidate.captured_at).await?;
            }
            return Ok(candidate.outcome(true, true));
        }
        let Some(id) = previous else {
            return Ok(ChargePayloadRecordOutcome::EvidenceExpired);
        };
        let winner = sqlx::query_as::<_, (DateTime<Utc>, Option<Uuid>)>(
            "SELECT captured_at,charge_session_id FROM panda.rivian_charge_payloads WHERE id=$1",
        )
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?;
        match winner {
            Some((captured_at, session)) => Ok(Candidate {
                id,
                captured_at,
                session,
            }
            .outcome(true, false)),
            None => self.expire(tx, Some(id)).await,
        }
    }
}
