use super::{DateTime, Duration, Utc, DEMO_ACTIVE_DAY_OFFSETS, DEMO_ADDRESSES};

pub(super) struct TripSlot {
    pub index: i32,
    pub route: usize,
    pub destination: usize,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub duration_minutes: i32,
    pub weather_samples: i32,
}

pub(super) fn trips(anchor: DateTime<Utc>, count: i32, weather: i32) -> Vec<TripSlot> {
    let mut slots = Vec::with_capacity(count.max(0) as usize);
    if count <= 0 {
        return slots;
    }
    let base_samples = weather / count;
    let extra_samples = weather % count;
    let days = DEMO_ACTIVE_DAY_OFFSETS.len();
    let routes = DEMO_ADDRESSES.len();
    for occurrence in 0..(count as usize).div_ceil(days) {
        for (day, offset) in DEMO_ACTIVE_DAY_OFFSETS.iter().enumerate() {
            let index = (occurrence * days + day) as i32;
            if index >= count {
                break;
            }
            let started_at = anchor - Duration::days(14)
                + Duration::days(*offset)
                + Duration::hours(8 + occurrence as i64 * 5);
            let duration_minutes = 18 + index % 5 * 7;
            slots.push(TripSlot {
                index,
                route: index as usize % routes,
                destination: (index as usize + 1) % routes,
                started_at,
                ended_at: started_at + Duration::minutes(i64::from(duration_minutes)),
                duration_minutes,
                weather_samples: (base_samples + i32::from(index >= count - extra_samples)).max(1),
            });
        }
    }
    slots
}

pub(super) struct ChargeSlot {
    pub index: i32,
    pub address: usize,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub duration: i32,
    pub kind: Option<&'static str>,
}

pub(super) fn charges(anchor: DateTime<Utc>, count: i32) -> impl Iterator<Item = ChargeSlot> {
    [
        (12, 180, Some("ac")),
        (8, 115, Some("ac")),
        (4, 46, Some("dc")),
        (1, 135, None),
    ]
    .into_iter()
    .enumerate()
    .cycle()
    .take(count.max(0) as usize)
    .enumerate()
    .map(move |(index, (address, (days, duration, kind)))| {
        let started_at = anchor - Duration::days(days) + Duration::hours(2);
        ChargeSlot {
            index: index as i32,
            address,
            started_at,
            ended_at: started_at + Duration::minutes(i64::from(duration)),
            duration,
            kind,
        }
    })
}

#[derive(sqlx::FromRow)]
pub(super) struct Period {
    pub state: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
}

pub(super) fn periods(anchors: Vec<Period>) -> Vec<Period> {
    let mut result = Vec::with_capacity(anchors.len() * 3);
    let mut previous: Option<DateTime<Utc>> = None;
    for anchor in anchors {
        if let Some(end) = previous {
            let ready_start = anchor.started_at - Duration::minutes(45);
            if anchor.started_at > end + Duration::minutes(45) {
                result.push(Period {
                    state: "sleep".into(),
                    started_at: end,
                    ended_at: Some(ready_start),
                });
            }
            if anchor.started_at > end {
                result.push(Period {
                    state: "ready".into(),
                    started_at: end.max(ready_start),
                    ended_at: Some(anchor.started_at),
                });
            }
        }
        previous = anchor.ended_at;
        if anchor.ended_at.is_some_and(|end| end > anchor.started_at) {
            result.push(anchor);
        }
    }
    result.sort_by_key(|period| period.started_at);
    result
}
