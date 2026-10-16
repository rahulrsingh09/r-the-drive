//! Read-only polling cadence derived from the current power-state observation.

use crate::models::telemetry::PowerState;
use std::time::Duration;

pub fn poll_interval(power_state: Option<&PowerState>) -> Duration {
    PollingMode::from_observation(power_state).interval()
}

enum PollingMode {
    Streaming,
    Charging,
    Awake,
    Dormant,
}

impl PollingMode {
    fn from_observation(state: Option<&PowerState>) -> Self {
        match state {
            Some(PowerState::Go | PowerState::Drive) => Self::Streaming,
            Some(PowerState::Charging) => Self::Charging,
            Some(PowerState::Ready) => Self::Awake,
            None | Some(PowerState::Sleep | PowerState::Unknown) => Self::Dormant,
        }
    }

    fn interval(self) -> Duration {
        Duration::from_secs(match self {
            Self::Streaming => 3600,
            Self::Charging => 30,
            Self::Awake => 300,
            Self::Dormant => 1800,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn charging_is_30s() {
        assert_eq!(
            poll_interval(Some(&PowerState::Charging)),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn sleep_is_30min() {
        assert_eq!(
            poll_interval(Some(&PowerState::Sleep)),
            Duration::from_secs(1800)
        );
    }

    #[test]
    fn provider_input_contract_covers_every_state_and_absence() {
        for (state, seconds) in [
            (None, 1800),
            (Some(PowerState::Unknown), 1800),
            (Some(PowerState::Sleep), 1800),
            (Some(PowerState::Ready), 300),
            (Some(PowerState::Charging), 30),
            (Some(PowerState::Go), 3600),
            (Some(PowerState::Drive), 3600),
        ] {
            assert_eq!(poll_interval(state.as_ref()), Duration::from_secs(seconds));
        }
    }

    #[test]
    fn provider_input_contract_is_stateless_across_transitions_and_duplicates() {
        for (raw, seconds) in [
            ("charging", 30),
            ("charging", 30),
            ("drive", 3600),
            ("ready", 300),
            ("future", 1800),
            ("GO", 3600),
            ("sleep", 1800),
        ] {
            let state = raw.parse::<PowerState>().unwrap();
            assert_eq!(poll_interval(Some(&state)), Duration::from_secs(seconds));
        }
    }
}
