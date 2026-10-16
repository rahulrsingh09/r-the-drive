use super::{CurveSampleRow, RawCurvePointRow};
use chrono::{DateTime, Utc};
use uuid::Uuid;

enum SocProjection {
    Midpoint(f64),
    Interpolated {
        first_ts: DateTime<Utc>,
        span_seconds: f64,
        start: f64,
        end: f64,
    },
}

impl SocProjection {
    fn at(&self, ts: DateTime<Utc>) -> f64 {
        match *self {
            Self::Midpoint(soc) => soc,
            Self::Interpolated {
                first_ts,
                span_seconds,
                start,
                end,
            } => {
                let ratio = ((ts - first_ts).num_seconds() as f64 / span_seconds).clamp(0.0, 1.0);
                start + (end - start) * ratio
            }
        }
    }
}

struct FallbackTimeline {
    started_at: DateTime<Utc>,
    span_minutes: f64,
    soc: SocProjection,
}

impl FallbackTimeline {
    fn locate(&self, ts: DateTime<Utc>) -> (f64, f64) {
        let elapsed_minutes = (ts - self.started_at).num_seconds() as f64 / 60.0;
        let soc = self.soc.at(ts);
        (elapsed_minutes.clamp(0.0, self.span_minutes), soc)
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build(
    session_id: Uuid,
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
    charger_type: Option<&str>,
    soc_start: f64,
    soc_end: f64,
    points: &[RawCurvePointRow],
    cap_kw: f64,
) -> Vec<CurveSampleRow> {
    let Some(first) = points.first() else {
        return Vec::new();
    };
    let span_minutes = (ended_at - started_at).num_seconds().max(1) as f64 / 60.0;
    let last = &points[points.len() - 1];
    let span_seconds = (last.ts - first.ts).num_seconds().max(1) as f64;
    let soc = if points.len() == 1 {
        SocProjection::Midpoint((soc_start + soc_end) / 2.0)
    } else {
        SocProjection::Interpolated {
            first_ts: first.ts,
            span_seconds,
            start: soc_start,
            end: soc_end,
        }
    };
    let timeline = FallbackTimeline {
        started_at,
        span_minutes,
        soc,
    };
    let mut samples = Vec::with_capacity(points.len());
    for point in points {
        let (minutes_elapsed, soc) = timeline.locate(point.ts);
        let charge_rate_kw = point.power_kw.unwrap_or(0.0).clamp(0.0, cap_kw);
        samples.push(CurveSampleRow {
            session_id,
            minutes_elapsed,
            charge_rate_kw,
            soc: Some(soc),
            sample_source: point.sample_source.clone(),
            charger_type: charger_type.map(ToOwned::to_owned),
            power_method: "recorded",
        });
    }
    samples
}
