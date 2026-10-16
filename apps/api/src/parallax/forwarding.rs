//! A bounded handoff never becomes a prerequisite for companion storage.
use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn project_and_forward(
    topic: &str,
    payload: &[u8],
    received_at: DateTime<Utc>,
    source_at: DateTime<Utc>,
    vehicle_id: Uuid,
    sender: Option<&mpsc::Sender<(String, TelemetryEvent)>>,
    report: &mut EnvelopeReport,
    capturing: bool,
) -> Result<Option<EnvelopeOutcome>> {
    if !is_canonical_telemetry_topic(topic) {
        return Ok(None);
    }
    report.forward_outcome = ForwardOutcome::NotEvaluated;
    let event = decode_vehicle_telemetry_with_notes(
        topic,
        payload,
        source_at,
        vehicle_id,
        &mut report.notes,
    )
    .map_err(|_| anyhow::anyhow!("typed Parallax decoder rejected topic {topic}"))?;
    let Some(event) = event else {
        report.forward_outcome = ForwardOutcome::NoCanonicalData;
        return Ok(Some(if report.notes.is_empty() {
            EnvelopeOutcome::Empty
        } else {
            EnvelopeOutcome::Ignored
        }));
    };
    if capturing {
        report.decoded = Some(ingestion_capture::present_fields(&event));
    }
    report.forward_outcome = deliver(topic, event, received_at, sender);
    Ok(Some(EnvelopeOutcome::Decoded))
}

fn deliver(
    topic: &str,
    event: TelemetryEvent,
    received_at: DateTime<Utc>,
    sender: Option<&mpsc::Sender<(String, TelemetryEvent)>>,
) -> ForwardOutcome {
    if event.ts < received_at - chrono::Duration::minutes(5)
        || event.ts > received_at + chrono::Duration::seconds(30)
    {
        return ForwardOutcome::Stale;
    }
    let Some(sender) = sender else {
        return ForwardOutcome::NotAttached;
    };
    let vehicle_id = event.vehicle_id;
    let error = match sender.try_send((topic.to_owned(), event)) {
        Ok(()) => return ForwardOutcome::Enqueued,
        Err(error) => error,
    };
    let (reason, outcome) = match error {
        mpsc::error::TrySendError::Full(_) => ("full", ForwardOutcome::Full),
        mpsc::error::TrySendError::Closed(_) => ("closed", ForwardOutcome::Closed),
    };
    let count = TELEMETRY_FORWARD_DROP_LOG_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
    tracing::debug!(vehicle_id=%vehicle_id, source=%topic, reason, "validated Parallax telemetry could not reach canonical worker");
    if count == 1 || count.is_power_of_two() {
        tracing::warn!(vehicle_id=%vehicle_id, source=%topic, reason, sampled_drop_count=count,
            "validated Parallax telemetry could not reach canonical worker");
    }
    outcome
}
