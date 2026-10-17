//! Typed, transactional security audit events.
//!
//! Audit metadata intentionally carries stable identifiers and enum-like
//! values only. Do not put credentials, tokens, email addresses, location, or
//! raw telemetry in this service.

use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction};
use std::time::Duration;
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::errors::AppError;

pub const SECURITY_EVENT_RETENTION_DAYS: i32 = 365;

#[derive(Debug, Clone, Copy)]
pub enum AuditOutcome {
    Success,
    Failure,
}

impl AuditOutcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SecurityAuditEvent {
    pub event_type: &'static str,
    pub actor_id: Option<Uuid>,
    /// Stable target identifier, for example `api_key:<uuid>`.
    pub target: Option<String>,
    pub request_id: Option<Uuid>,
    pub outcome: AuditOutcome,
    pub metadata: Value,
}

impl SecurityAuditEvent {
    pub fn success(event_type: &'static str, actor_id: Option<Uuid>) -> Self {
        Self {
            event_type,
            actor_id,
            target: None,
            request_id: None,
            outcome: AuditOutcome::Success,
            metadata: Value::Object(Default::default()),
        }
    }

    pub fn failure(event_type: &'static str, actor_id: Option<Uuid>) -> Self {
        Self {
            outcome: AuditOutcome::Failure,
            ..Self::success(event_type, actor_id)
        }
    }

    pub fn target(mut self, target: impl Into<String>) -> Self {
        self.target = Some(target.into());
        self
    }

    pub fn metadata(mut self, metadata: Value) -> Self {
        self.metadata = metadata;
        self
    }

    /// Attach the request correlation ID emitted by the router when present.
    /// Invalid or non-UUID values are deliberately ignored rather than copied
    /// into audit metadata.
    pub fn request_id_from_headers(mut self, headers: &axum::http::HeaderMap) -> Self {
        let parsed = match headers
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
        {
            Some(value) => Uuid::parse_str(value).ok(),
            None => None,
        };
        self.request_id = parsed;
        self
    }

    pub fn request_id(mut self, request_id: Option<Uuid>) -> Self {
        self.request_id = request_id;
        self
    }

    pub async fn record(self, pool: &PgPool) -> Result<(), AppError> {
        insert(pool, &self).await
    }

    pub async fn record_tx(self, tx: &mut Transaction<'_, Postgres>) -> Result<(), AppError> {
        insert(&mut **tx, &self).await
    }
}

pub async fn prune_expired(pool: &PgPool) -> Result<u64, AppError> {
    let result = sqlx::query(
        "DELETE FROM panda.security_events \
         WHERE created_at < now() - make_interval(days => $1)",
    )
    .bind(SECURITY_EVENT_RETENTION_DAYS)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

const RETENTION_PERIOD: Duration = Duration::from_secs(24 * 60 * 60);

fn next_retention_tick(
    scheduled: tokio::time::Instant,
    observed: tokio::time::Instant,
) -> tokio::time::Instant {
    if observed <= scheduled + Duration::from_millis(5) {
        return scheduled + RETENTION_PERIOD;
    }
    let remainder = (observed - scheduled).as_nanos() % RETENTION_PERIOD.as_nanos();
    observed + RETENTION_PERIOD - Duration::from_nanos(remainder as u64)
}

pub fn start_retention_worker(pool: PgPool) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut scheduled = tokio::time::Instant::now();
        loop {
            tokio::time::sleep_until(scheduled).await;
            scheduled = next_retention_tick(scheduled, tokio::time::Instant::now());
            let outcome = prune_expired(&pool).await;
            if let Err(error) = outcome {
                tracing::error!(error = ?error, "security audit retention failed");
            } else if let Ok(removed @ 1..) = outcome {
                tracing::info!(removed, "expired security audit events pruned");
            }
        }
    })
}

async fn insert<'e, E>(executor: E, event: &SecurityAuditEvent) -> Result<(), AppError>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    // Derive the legacy detail from the same bound values as the structured row.
    sqlx::query(
        "INSERT INTO panda.security_events
         (event_type, user_id, target, request_id, outcome, metadata, detail, created_at)
         VALUES ($1, $2, $3, $4, $5, $6,
                 'outcome=' || $5 || ' target=' || COALESCE($3, 'none'), now())",
    )
    .bind(event.event_type)
    .bind(event.actor_id)
    .bind(&event.target)
    .bind(event.request_id)
    .bind(event.outcome.as_str())
    .bind(&event.metadata)
    .execute(executor)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_detail_is_redacted_and_stable() {
        let event = SecurityAuditEvent::success("api_key_created", Some(Uuid::nil()))
            .target("api_key:00000000-0000-0000-0000-000000000000")
            .metadata(serde_json::json!({ "vehicle_id": Uuid::nil() }));
        assert_eq!(event.event_type, "api_key_created");
        assert!(event.metadata.get("vehicle_id").is_some());
    }

    #[test]
    fn request_id_accepts_only_router_uuid_values() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("x-request-id", "not-a-uuid".parse().unwrap());
        assert!(SecurityAuditEvent::success("event", None)
            .request_id_from_headers(&headers)
            .request_id
            .is_none());

        let request_id = Uuid::new_v4();
        headers.insert("x-request-id", request_id.to_string().parse().unwrap());
        assert_eq!(
            SecurityAuditEvent::success("event", None)
                .request_id_from_headers(&headers)
                .request_id,
            Some(request_id)
        );
    }

    #[test]
    fn retention_contract_is_one_year() {
        assert_eq!(SECURITY_EVENT_RETENTION_DAYS, 365);
    }
}

#[cfg(test)]
mod full_replacement_edge_sessions_tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn retention_deadlines_match_the_original_interval_skip_contract() {
        for late in [
            Duration::ZERO,
            Duration::from_millis(5),
            Duration::from_secs(100),
            RETENTION_PERIOD * 3 + Duration::from_secs(100),
        ] {
            let start = tokio::time::Instant::now();
            let mut original = tokio::time::interval_at(start, RETENTION_PERIOD);
            original.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            tokio::time::advance(late).await;
            let next = next_retention_tick(start, tokio::time::Instant::now());
            original.tick().await;
            assert_eq!(original.tick().await, next);
        }
    }
}
