use super::*;

pub(super) enum Admission {
    YieldInteractive,
    Wait(Duration),
    Granted(u64),
}

impl Admission {
    pub(super) fn reserve(state: &mut NominatimState, now: Instant, base_ms: u64) -> Self {
        let wait = state.next_allowed.saturating_duration_since(now);
        if !wait.is_zero() {
            return Self::Wait(wait);
        }
        let interval = base_ms + state.adaptive_extra_ms;
        state.next_allowed = now + Duration::from_millis(interval);
        Self::Granted(interval)
    }
}

pub(super) struct QueueProgress {
    lane: NominatimLane,
    started: Instant,
    gate_wait: Duration,
}

impl QueueProgress {
    pub(super) fn new(lane: NominatimLane) -> Self {
        Self {
            lane,
            started: Instant::now(),
            gate_wait: Duration::ZERO,
        }
    }
    pub(super) fn wait_for(&mut self, duration: Duration) {
        self.gate_wait += duration;
    }
    pub(super) fn finish(self, interval: u64) -> NominatimSlot {
        NominatimSlot {
            lane: self.lane,
            queued_ms: self.started.elapsed().as_millis() as u64,
            gate_wait_ms: self.gate_wait.as_millis() as u64,
            effective_interval_ms: interval,
        }
    }
}

pub(super) enum RateAdjustment {
    Unchanged,
    Recover,
    Backoff,
    RetryAfter(u64),
}

impl RateAdjustment {
    pub(super) fn from_response(status: Option<u16>, retry: Option<Duration>) -> Self {
        match status {
            Some(429) => retry
                .map(|duration| Self::RetryAfter(duration.as_millis() as u64))
                .unwrap_or(Self::Backoff),
            Some(200..=299) => Self::Recover,
            _ => Self::Unchanged,
        }
    }
    pub(super) fn apply(self, extra: u64, base: u64, maximum: u64) -> u64 {
        match self {
            Self::Unchanged => extra,
            Self::Recover => extra.saturating_sub(200),
            Self::Backoff => if extra == 0 {
                500
            } else {
                extra.saturating_mul(2)
            }
            .min(maximum),
            Self::RetryAfter(retry) => extra.max(retry.saturating_sub(base)).min(maximum),
        }
    }
}
