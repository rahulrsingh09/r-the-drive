use chrono::{DateTime, Duration, NaiveDate, TimeZone, Timelike, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TouPeriod {
    pub label: String,
    pub start_minute: u16,
    pub end_minute: u16,
    pub rate: f64,
}

/// A user-defined rate card for computing the cost of a charging session.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct CostProfile {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub billing_type: String,
    pub rate: f64,
    pub session_fee: f64,
    pub currency: String,
    pub timezone: Option<String>,
    pub tou_periods: serde_json::Value,
    pub effective_from: Option<NaiveDate>,
    pub effective_to: Option<NaiveDate>,
    pub created_at: DateTime<Utc>,
}

impl CostProfile {
    pub fn tou_periods(&self) -> Vec<TouPeriod> {
        serde_json::from_value(self.tou_periods.clone()).unwrap_or_default()
    }

    pub fn is_effective_at(&self, started_at: DateTime<Utc>) -> bool {
        let day = self
            .timezone
            .as_deref()
            .and_then(|name| name.parse::<Tz>().ok())
            .map_or_else(
                || started_at.date_naive(),
                |tz| started_at.with_timezone(&tz).date_naive(),
            );

        self.effective_from.is_none_or(|first| day >= first)
            && self.effective_to.is_none_or(|last| day <= last)
    }
}

/// A single measured energy delivery interval used for accurate TOU cost
/// attribution. Each point represents the kWh actually delivered at a specific
/// timestamp (typically one 1-minute telemetry bucket).
#[derive(Debug, Clone)]
pub struct TimedEnergyPoint {
    pub ts: DateTime<Utc>,
    pub kwh: f64,
}

pub fn validate_tou_periods(periods: &[TouPeriod]) -> Result<(), String> {
    if periods.is_empty() {
        return Err("TOU schedules need at least one period.".into());
    }

    let mut expected_start = 0u16;
    for period in periods {
        if period.label.trim().is_empty() {
            return Err("Each TOU period needs a label.".into());
        }
        if !period.rate.is_finite() || period.rate < 0.0 {
            return Err("Each TOU period needs a non-negative rate.".into());
        }
        // Sanity check: rates are stored in $/kWh.  Real US residential
        // peak rates max out around $0.60/kWh.  Anything above $5 almost
        // certainly means the user typed cents-per-kWh (e.g. "32.2499")
        // by mistake — reject at the API boundary so it can't poison
        // every cost calculation against this profile.
        if period.rate > 5.0 {
            return Err(format!(
                "Rate {:.4} $/kWh is too high. Rates must be entered in dollars per kWh \
                 (e.g. 0.32 for 32¢/kWh), not cents.",
                period.rate
            ));
        }
        if period.start_minute != expected_start {
            return Err("TOU periods must be contiguous and start at 00:00.".into());
        }
        if period.end_minute <= period.start_minute {
            return Err("Each TOU period must end after it starts.".into());
        }
        expected_start = period.end_minute;
    }

    if expected_start != 24 * 60 {
        return Err("TOU periods must cover the full day through 24:00.".into());
    }

    Ok(())
}

/// Calculate a profile amount, including its fee, when the start date is eligible.
pub fn compute_cost(
    profile: &CostProfile,
    energy_added_kwh: Option<f64>,
    energy_used_kwh: Option<f64>,
    duration_minutes: i32,
    started_at: DateTime<Utc>,
    ended_at: Option<DateTime<Utc>>,
) -> Option<f64> {
    if !profile.is_effective_at(started_at) {
        return None;
    }
    let quantity = match profile.billing_type.as_str() {
        "per_kwh" => BillableQuantity::Energy(billed_energy(energy_added_kwh, energy_used_kwh)?),
        "per_minute" => BillableQuantity::Minutes(duration_minutes),
        "flat" => BillableQuantity::Session,
        "free" => BillableQuantity::Free,
        "tou" => {
            let energy = billed_energy(energy_added_kwh, energy_used_kwh)?;
            let timezone = profile_timezone(profile)?;
            let window = SessionWindow::new(timezone, started_at, ended_at, duration_minutes)?;
            let schedule = DailySchedule::new(timezone, profile.tou_periods())?;
            BillableQuantity::Scheduled(window.price(&schedule, energy)?)
        }
        _ => return None,
    };
    Some(quantity.price(profile.rate, profile.session_fee))
}

enum BillableQuantity {
    Energy(f64),
    Minutes(i32),
    Session,
    Free,
    Scheduled(f64),
}

impl BillableQuantity {
    fn price(self, rate: f64, fee: f64) -> f64 {
        match self {
            Self::Energy(kwh) => rate * kwh + fee,
            Self::Minutes(minutes) => rate * minutes as f64 + fee,
            Self::Session => rate + fee,
            Self::Free => 0.0,
            Self::Scheduled(amount) => amount + fee,
        }
    }
}

fn billed_energy(added: Option<f64>, used: Option<f64>) -> Option<f64> {
    let energy = f64::max(added.unwrap_or(0.0), used.unwrap_or(0.0));
    if energy <= 0.0 {
        None
    } else {
        Some(energy)
    }
}

fn profile_timezone(profile: &CostProfile) -> Option<Tz> {
    profile.timezone.as_deref().unwrap_or("UTC").parse().ok()
}

/// Price measured intervals using their local minute and the first matching period.
/// Empty readings, missing periods or an invalid timezone return `None`.
/// The caller owns effective-date selection and the readings' session boundaries.
pub fn compute_tou_cost_from_readings(
    profile: &CostProfile,
    readings: &[TimedEnergyPoint],
) -> Option<f64> {
    if readings.is_empty() {
        return None;
    }
    let schedule = DailySchedule::new(profile_timezone(profile)?, profile.tou_periods())?;
    let amount: f64 = readings
        .iter()
        .map(|point| point.kwh * schedule.rate_at(point.ts))
        .sum();
    Some(amount + profile.session_fee)
}

struct DailySchedule {
    timezone: Tz,
    periods: Vec<TouPeriod>,
}

impl DailySchedule {
    fn new(timezone: Tz, periods: Vec<TouPeriod>) -> Option<Self> {
        if periods.is_empty() {
            None
        } else {
            Some(Self { timezone, periods })
        }
    }

    fn rate_at(&self, timestamp: DateTime<Utc>) -> f64 {
        let local = timestamp.with_timezone(&self.timezone);
        let minute = local.hour() * 60 + local.minute();
        self.periods
            .iter()
            .find(|period| {
                u32::from(period.start_minute) <= minute && minute < u32::from(period.end_minute)
            })
            .map_or(0.0, |period| period.rate)
    }

    fn intervals_on(&self, day: NaiveDate) -> impl Iterator<Item = TariffInterval> + '_ {
        self.periods.iter().filter_map(move |period| {
            Some(TariffInterval {
                start: local_boundary(self.timezone, day, period.start_minute)?,
                end: local_boundary(self.timezone, day, period.end_minute)?,
                rate: period.rate,
            })
        })
    }
}

struct TariffInterval {
    start: DateTime<Tz>,
    end: DateTime<Tz>,
    rate: f64,
}

struct SessionWindow {
    start: DateTime<Tz>,
    end: DateTime<Tz>,
}

impl SessionWindow {
    fn new(
        timezone: Tz,
        start: DateTime<Utc>,
        end: Option<DateTime<Utc>>,
        duration_minutes: i32,
    ) -> Option<Self> {
        let end =
            end.unwrap_or_else(|| start + Duration::minutes(i64::from(duration_minutes.max(0))));
        if end <= start {
            return None;
        }
        Some(Self {
            start: start.with_timezone(&timezone),
            end: end.with_timezone(&timezone),
        })
    }

    fn overlap_minutes(&self, interval: &TariffInterval) -> Option<f64> {
        let start = self.start.max(interval.start);
        let end = self.end.min(interval.end);
        (end > start).then(|| (end - start).num_seconds() as f64 / 60.0)
    }

    fn price(&self, schedule: &DailySchedule, energy: f64) -> Option<f64> {
        let days = std::iter::successors(Some(self.start.date_naive()), |day| day.succ_opt())
            .take_while(|day| *day <= self.end.date_naive());
        let mut minutes = 0.0;
        let mut rate_minutes = 0.0;
        for interval in days.flat_map(|day| schedule.intervals_on(day)) {
            if let Some(overlap) = self.overlap_minutes(&interval) {
                minutes += overlap;
                rate_minutes += overlap * interval.rate;
            }
        }
        if minutes <= 0.0 {
            None
        } else {
            Some(energy * (rate_minutes / minutes))
        }
    }
}

fn local_boundary(timezone: Tz, day: NaiveDate, minute: u16) -> Option<DateTime<Tz>> {
    let minute = minute.min(24 * 60);
    let (day, minute) = if minute == 24 * 60 {
        (day.succ_opt()?, 0)
    } else {
        (day, minute)
    };
    let naive = day.and_hms_opt(u32::from(minute / 60), u32::from(minute % 60), 0)?;
    // Earliest occurrence preserves repeated-hour allocation; nonexistent boundaries are skipped.
    timezone.from_local_datetime(&naive).earliest()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn profile(billing_type: &str, rate: f64, session_fee: f64) -> CostProfile {
        CostProfile {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            name: "Test".into(),
            billing_type: billing_type.into(),
            rate,
            session_fee,
            currency: "USD".into(),
            timezone: Some("UTC".into()),
            tou_periods: json!([]),
            effective_from: None,
            effective_to: None,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn test_per_kwh() {
        let p = profile("per_kwh", 0.13, 0.0);
        let cost = compute_cost(&p, Some(50.0), None, 60, Utc::now(), None).unwrap();
        assert!((cost - 6.5).abs() < 0.001);
    }

    #[test]
    fn test_per_kwh_uses_greatest() {
        // energy_used_kwh > energy_added_kwh → use energy_used_kwh
        let p = profile("per_kwh", 0.13, 1.0);
        let cost = compute_cost(&p, Some(48.0), Some(52.0), 60, Utc::now(), None).unwrap();
        assert!((cost - (0.13 * 52.0 + 1.0)).abs() < 0.001);
    }

    #[test]
    fn test_per_minute() {
        let p = profile("per_minute", 0.05, 2.0);
        let cost = compute_cost(&p, None, None, 60, Utc::now(), None).unwrap();
        assert!((cost - 5.0).abs() < 0.001);
    }

    #[test]
    fn test_free() {
        let p = profile("free", 0.0, 0.0);
        let cost = compute_cost(&p, Some(30.0), None, 30, Utc::now(), None).unwrap();
        assert_eq!(cost, 0.0);
    }

    #[test]
    fn test_flat() {
        let p = profile("flat", 5.0, 1.5);
        let cost = compute_cost(&p, None, None, 20, Utc::now(), None).unwrap();
        assert!((cost - 6.5).abs() < 0.001);
    }

    #[test]
    fn rejects_profiles_outside_effective_window() {
        let mut p = profile("per_kwh", 0.13, 0.0);
        p.effective_from = Some((Utc::now() + Duration::days(1)).date_naive());
        assert!(compute_cost(&p, Some(10.0), None, 30, Utc::now(), None).is_none());
    }

    #[test]
    fn test_per_kwh_no_energy_returns_none() {
        let p = profile("per_kwh", 0.13, 0.0);
        assert!(compute_cost(&p, None, None, 30, Utc::now(), None).is_none());
    }

    #[test]
    fn validate_tou_rejects_cents_typed_as_dollars() {
        // The exact regression: user typed "32.2499" meaning ¢/kWh.
        let bad = vec![
            TouPeriod {
                label: "Night".into(),
                start_minute: 0,
                end_minute: 360,
                rate: 0.0,
            },
            TouPeriod {
                label: "Day".into(),
                start_minute: 360,
                end_minute: 1200,
                rate: 32.2499,
            },
            TouPeriod {
                label: "Night2".into(),
                start_minute: 1200,
                end_minute: 1440,
                rate: 0.0,
            },
        ];
        let err = validate_tou_periods(&bad).expect_err("should reject cents-as-dollars");
        assert!(
            err.contains("too high"),
            "error should mention rate is too high, got: {err}"
        );
    }

    #[test]
    fn validate_tou_accepts_realistic_peak_rate() {
        // 32¢/kWh peak — a realistic California/Texas summer rate.
        let good = vec![
            TouPeriod {
                label: "Off-peak".into(),
                start_minute: 0,
                end_minute: 360,
                rate: 0.10,
            },
            TouPeriod {
                label: "Peak".into(),
                start_minute: 360,
                end_minute: 1200,
                rate: 0.322499,
            },
            TouPeriod {
                label: "Off-peak".into(),
                start_minute: 1200,
                end_minute: 1440,
                rate: 0.10,
            },
        ];
        assert!(validate_tou_periods(&good).is_ok());
    }

    #[test]
    fn test_tou_weighted_rate() {
        let mut p = profile("tou", 0.0, 1.0);
        p.timezone = Some("UTC".into());
        p.tou_periods = json!([
            { "label": "Off-peak", "start_minute": 0, "end_minute": 60, "rate": 0.10 },
            { "label": "Peak", "start_minute": 60, "end_minute": 120, "rate": 0.30 },
            { "label": "Overnight", "start_minute": 120, "end_minute": 1440, "rate": 0.10 }
        ]);

        let start = Utc.with_ymd_and_hms(2026, 1, 1, 0, 30, 0).single().unwrap();
        let end = Utc.with_ymd_and_hms(2026, 1, 1, 1, 30, 0).single().unwrap();
        let cost = compute_cost(&p, Some(10.0), None, 60, start, Some(end)).unwrap();

        assert!((cost - 3.0).abs() < 0.001);
    }

    #[test]
    fn test_tou_zero_cost_overnight_in_local_timezone() {
        let mut p = profile("tou", 0.0, 0.0);
        p.timezone = Some("America/Chicago".into());
        p.tou_periods = json!([
            { "label": "Overnight", "start_minute": 0, "end_minute": 360, "rate": 0.0 },
            { "label": "Daytime", "start_minute": 360, "end_minute": 1200, "rate": 32.2499 },
            { "label": "Evening", "start_minute": 1200, "end_minute": 1440, "rate": 0.0 }
        ]);

        let start = Utc
            .with_ymd_and_hms(2026, 5, 12, 4, 53, 0)
            .single()
            .unwrap();
        let end = Utc
            .with_ymd_and_hms(2026, 5, 12, 10, 23, 0)
            .single()
            .unwrap();
        let cost = compute_cost(&p, Some(46.51), None, 330, start, Some(end)).unwrap();

        assert_eq!(cost, 0.0);
    }
}
