use super::{
    finite_value, normalize_capacity_wh, percentile, TripPowerData, TripPowerInterval,
    TripPowerMetadata, TripPowerSource, TripPowerTelemetryRow,
};
use chrono::{DateTime, Utc};

enum PowerEvidence {
    Direct(usize),
    Capacity(f64),
    Unavailable,
}

impl PowerEvidence {
    fn select(rows: &[TripPowerTelemetryRow]) -> Self {
        let direct_count = rows.iter().fold(0, |count, row| {
            count
                + usize::from(
                    finite_value(row.power_kw).is_some()
                        || finite_value(row.regen_power_kw).is_some(),
                )
        });
        if direct_count >= 2 {
            return Self::Direct(direct_count);
        }
        let capacities: Vec<_> = rows
            .iter()
            .filter_map(|row| normalize_capacity_wh(row.battery_capacity_wh))
            .collect();
        match percentile(&capacities, 0.5) {
            Some(capacity_wh) => Self::Capacity(capacity_wh),
            None => Self::Unavailable,
        }
    }
}

#[derive(Clone, Copy)]
struct SocPoint {
    ts: DateTime<Utc>,
    percent: f64,
}

#[derive(Default)]
struct SocCursor {
    retained: Option<SocPoint>,
}

impl SocCursor {
    fn advance(&mut self, row: &TripPowerTelemetryRow) -> Option<SocChange> {
        let percent =
            finite_value(row.battery_level).filter(|value| (0.0..=100.0).contains(value))?;
        if let Some(previous) = self.retained {
            if (percent - previous.percent).abs() <= 1e-6 {
                return None;
            }
        }
        let current = SocPoint {
            ts: row.ts,
            percent,
        };
        let previous = self.retained.replace(current)?;
        Some(SocChange::between(previous, current))
    }
}

struct SocChange {
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
    elapsed_seconds: f64,
    percent_used: f64,
}

impl SocChange {
    fn between(start: SocPoint, end: SocPoint) -> Self {
        Self {
            started_at: start.ts,
            ended_at: end.ts,
            elapsed_seconds: (end.ts - start.ts).num_milliseconds() as f64 / 1_000.0,
            percent_used: start.percent - end.percent,
        }
    }

    fn power_interval(&self, capacity_wh: f64) -> Option<TripPowerInterval> {
        if !(2.0..=300.0).contains(&self.elapsed_seconds)
            || !(0.05..=0.5).contains(&self.percent_used.abs())
        {
            return None;
        }
        let net_power_kw =
            (self.percent_used / 100.0 * capacity_wh) / (self.elapsed_seconds / 3_600.0) / 1_000.0;
        if !net_power_kw.is_finite() || net_power_kw.abs() > 750.0 {
            return None;
        }
        Some(TripPowerInterval {
            started_at: self.started_at,
            ended_at: self.ended_at,
            net_power_kw,
        })
    }
}

#[derive(Default)]
struct EstimatedIntervals {
    intervals: Vec<TripPowerInterval>,
    seconds: Vec<f64>,
}

impl EstimatedIntervals {
    fn collect(rows: &[TripPowerTelemetryRow], capacity_wh: f64) -> Self {
        let mut cursor = SocCursor::default();
        let mut estimates = Self::default();
        for row in rows {
            let Some(change) = cursor.advance(row) else {
                continue;
            };
            if let Some(interval) = change.power_interval(capacity_wh) {
                estimates.intervals.push(interval);
                estimates.seconds.push(change.elapsed_seconds);
            }
        }
        estimates
    }

    fn summarize(self, trip_duration_seconds: i32) -> TripPowerData {
        if self.intervals.is_empty() {
            return without_estimates(TripPowerSource::Unavailable, 0);
        }
        let covered_seconds = self.seconds.iter().sum::<f64>();
        let duration_seconds = f64::from(trip_duration_seconds.max(1));
        TripPowerData {
            metadata: TripPowerMetadata {
                source: TripPowerSource::EstimatedSoc,
                sample_count: self.intervals.len(),
                median_interval_seconds: percentile(&self.seconds, 0.5),
                p90_interval_seconds: percentile(&self.seconds, 0.9),
                coverage_percent: Some(
                    (covered_seconds / duration_seconds * 100.0).clamp(0.0, 100.0),
                ),
            },
            intervals: self.intervals,
        }
    }
}

fn without_estimates(source: TripPowerSource, sample_count: usize) -> TripPowerData {
    TripPowerData {
        metadata: TripPowerMetadata {
            source,
            sample_count,
            median_interval_seconds: None,
            p90_interval_seconds: None,
            coverage_percent: None,
        },
        intervals: Vec::new(),
    }
}

pub(super) fn derive(rows: &[TripPowerTelemetryRow], trip_duration_seconds: i32) -> TripPowerData {
    match PowerEvidence::select(rows) {
        PowerEvidence::Direct(count) => without_estimates(TripPowerSource::Direct, count),
        PowerEvidence::Unavailable => without_estimates(TripPowerSource::Unavailable, 0),
        PowerEvidence::Capacity(capacity_wh) => {
            EstimatedIntervals::collect(rows, capacity_wh).summarize(trip_duration_seconds)
        }
    }
}
