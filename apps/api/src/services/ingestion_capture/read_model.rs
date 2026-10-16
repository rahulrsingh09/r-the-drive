use super::*;

/// The capture's state as the Settings page shows it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CaptureStatus {
    pub state: &'static str,
    pub started_at: Option<DateTime<Utc>>,
    pub ends_at: Option<DateTime<Utc>>,
    pub stopped_at: Option<DateTime<Utc>>,
    pub stop_reason: Option<String>,
    pub event_count: i64,
    pub last_event_at: Option<DateTime<Utc>>,
    pub truncated: bool,
}

#[derive(sqlx::FromRow)]
struct CaptureRow {
    capture_id: Uuid,
    started_at: DateTime<Utc>,
    enabled_until: DateTime<Utc>,
    stopped_at: Option<DateTime<Utc>>,
    stop_reason: Option<String>,
    dropped_events: i64,
}

async fn load_row(pool: &PgPool, vehicle_id: Uuid) -> sqlx::Result<Option<CaptureRow>> {
    sqlx::query_as::<_, CaptureRow>(
        "SELECT capture_id, started_at, enabled_until, stopped_at, stop_reason, dropped_events \
         FROM panda.vehicle_ingestion_diagnostics WHERE vehicle_id = $1",
    )
    .bind(vehicle_id)
    .fetch_optional(pool)
    .await
}

struct EventStats {
    count: i64,
    last_event_at: Option<DateTime<Utc>>,
    truncated: bool,
    counts_by_kind: Vec<(String, i64)>,
}

async fn event_stats(pool: &PgPool, capture_id: Uuid) -> sqlx::Result<EventStats> {
    let groups = sqlx::query_as::<_, (String, i64, Option<DateTime<Utc>>)>(
        "SELECT kind, count(*), max(recorded_at) \
         FROM panda.vehicle_ingestion_capture_events WHERE capture_id = $1 \
         GROUP BY kind ORDER BY kind",
    )
    .bind(capture_id)
    .fetch_all(pool)
    .await?;
    let mut stats = EventStats {
        count: 0,
        last_event_at: None,
        truncated: false,
        counts_by_kind: Vec::with_capacity(groups.len()),
    };
    for (kind, count, last) in groups {
        if !matches!(
            kind.as_str(),
            "capture_started" | "capture_stopped" | "truncated"
        ) {
            stats.count += count;
        }
        stats.last_event_at = stats.last_event_at.max(last);
        stats.truncated |= kind == "truncated";
        stats.counts_by_kind.push((kind, count));
    }
    Ok(stats)
}

pub async fn status(pool: &PgPool, vehicle_id: Uuid) -> sqlx::Result<CaptureStatus> {
    let Some(row) = load_row(pool, vehicle_id).await? else {
        return Ok(CaptureStatus {
            state: "idle",
            started_at: None,
            ends_at: None,
            stopped_at: None,
            stop_reason: None,
            event_count: 0,
            last_event_at: None,
            truncated: false,
        });
    };
    let stats = event_stats(pool, row.capture_id).await?;
    let running = row.stopped_at.is_none() && row.enabled_until > Utc::now();
    Ok(CaptureStatus {
        state: if running { "capturing" } else { "stopped" },
        started_at: Some(row.started_at),
        ends_at: Some(row.enabled_until),
        stopped_at: row.stopped_at.or((!running).then_some(row.enabled_until)),
        stop_reason: row
            .stop_reason
            .or((!running).then(|| StopReason::Expired.as_str().to_owned())),
        event_count: stats.count,
        last_event_at: stats.last_event_at,
        truncated: stats.truncated,
    })
}

/// A finished or running capture, ready to stream as NDJSON.
pub struct Export {
    pub header: Value,
    pub filename: String,
    pub capture_id: Uuid,
}

/// Build the export header for the vehicle's latest capture, or `None` when
/// there is nothing to download.
pub async fn export(pool: &PgPool, vehicle_id: Uuid) -> sqlx::Result<Option<Export>> {
    let Some(row) = load_row(pool, vehicle_id).await? else {
        return Ok(None);
    };
    let stats = event_stats(pool, row.capture_id).await?;
    let model =
        sqlx::query_scalar::<_, Option<String>>("SELECT model FROM panda.vehicles WHERE id = $1")
            .bind(vehicle_id)
            .fetch_optional(pool)
            .await?
            .flatten();
    let running = row.stopped_at.is_none() && row.enabled_until > Utc::now();
    let header = build_header(
        model.as_deref(),
        row.started_at,
        (!running).then(|| row.stopped_at.unwrap_or(row.enabled_until)),
        row.stop_reason.as_deref(),
        &stats.counts_by_kind,
        stats.count,
        row.dropped_events,
        stats.truncated,
    );
    Ok(Some(Export {
        filename: export_filename(model.as_deref(), row.started_at),
        header,
        capture_id: row.capture_id,
    }))
}

/// One page of exported rows, in capture order, without the vehicle id.
pub async fn export_rows(
    pool: &PgPool,
    capture_id: Uuid,
    after_id: i64,
    limit: i64,
) -> sqlx::Result<Vec<(i64, Value)>> {
    let rows = sqlx::query_as::<_, (i64, DateTime<Utc>, String, Value)>(
        "SELECT id, recorded_at, kind, fields FROM panda.vehicle_ingestion_capture_events \
         WHERE capture_id = $1 AND id > $2 ORDER BY id LIMIT $3",
    )
    .bind(capture_id)
    .bind(after_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, recorded_at, kind, fields)| (id, export_line(recorded_at, &kind, fields)))
        .collect())
}

fn export_line(recorded_at: DateTime<Utc>, kind: &str, fields: Value) -> Value {
    serde_json::json!({
        "recorded_at": recorded_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "kind": kind,
        "fields": sanitize(fields),
    })
}

pub fn export_filename(model: Option<&str>, started_at: DateTime<Utc>) -> String {
    let model: String = model
        .unwrap_or("vehicle")
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_ascii_lowercase();
    let model = if model.is_empty() {
        "vehicle".to_owned()
    } else {
        model
    };
    format!(
        "panda-capture-{model}-{}.jsonl",
        started_at.format("%Y%m%dT%H%MZ")
    )
}

#[allow(clippy::too_many_arguments)] // Header fields are independent capture facts.
fn build_header(
    model: Option<&str>,
    started_at: DateTime<Utc>,
    stopped_at: Option<DateTime<Utc>>,
    stop_reason: Option<&str>,
    counts_by_kind: &[(String, i64)],
    event_count: i64,
    dropped_events: i64,
    truncated: bool,
) -> Value {
    let counts: Map<String, Value> = counts_by_kind
        .iter()
        .map(|(kind, count)| (kind.clone(), Value::from(*count)))
        .collect();
    serde_json::json!({
        "kind": "header",
        "format_version": FORMAT_VERSION,
        "app_version": std::env::var("PANDA_BUILD_VERSION")
            .unwrap_or_else(|_| env!("CARGO_PKG_VERSION").into()),
        "model": model,
        "started_at": started_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "stopped_at": stopped_at.map(|at| at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
        "stop_reason": stop_reason,
        "event_count": event_count,
        "counts_by_kind": counts,
        "dropped_events": dropped_events,
        "truncated": truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn export_lines_omit_vehicle_id_and_coordinates() {
        let line = export_line(
            "2026-09-30T21:10:47.988Z".parse().unwrap(),
            "parallax_envelope",
            json!({ "vehicle_id": "x", "decoded": { "latitude": 1.0, "speed_mph": 0.0 } }),
        );
        assert_eq!(
            line,
            json!({
                "recorded_at": "2026-09-30T21:10:47.988Z",
                "kind": "parallax_envelope",
                "fields": { "decoded": { "speed_mph": 0.0 } },
            })
        );
    }

    #[test]
    fn present_fields_drops_nulls_and_identity() {
        #[derive(serde::Serialize)]
        struct Sample {
            vehicle_id: Uuid,
            latitude: Option<f64>,
            speed_mph: Option<f64>,
            power_state: Option<&'static str>,
        }
        let value = present_fields(&Sample {
            vehicle_id: Uuid::nil(),
            latitude: Some(1.0),
            speed_mph: None,
            power_state: Some("ready"),
        });
        assert_eq!(value, json!({ "power_state": "ready" }));
    }

    #[test]
    fn header_is_shareable_and_complete() {
        let header = build_header(
            Some("R1S"),
            "2026-09-30T21:10:00Z".parse().unwrap(),
            Some("2026-09-30T21:25:00Z".parse().unwrap()),
            Some("user"),
            &[
                ("legacy_frame".into(), 12),
                ("parallax_envelope".into(), 30),
            ],
            42,
            0,
            false,
        );
        assert_eq!(header["kind"], "header");
        assert_eq!(header["format_version"], FORMAT_VERSION);
        assert_eq!(header["model"], "R1S");
        assert_eq!(header["stopped_at"], "2026-09-30T21:25:00.000Z");
        assert_eq!(header["counts_by_kind"]["parallax_envelope"], 30);
        assert_eq!(header["event_count"], 42);
        let keys: Vec<&str> = header
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert!(!keys
            .iter()
            .any(|key| key.contains("vehicle") || *key == "vin" || *key == "name"));
    }

    #[test]
    fn filename_uses_model_and_start_time() {
        assert_eq!(
            export_filename(Some("R1S"), "2026-09-30T21:10:12Z".parse().unwrap()),
            "panda-capture-r1s-20260930T2110Z.jsonl"
        );
        assert_eq!(
            export_filename(None, "2026-09-30T21:10:12Z".parse().unwrap()),
            "panda-capture-vehicle-20260930T2110Z.jsonl"
        );
    }
}
