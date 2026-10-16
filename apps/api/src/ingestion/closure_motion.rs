//! Live opening/closing state for closures.
//!
//! Parallax reports a closure in motion as AJAR, OPENING, or CLOSING. The
//! stored closure fields only say "closed" or "not closed", so this tracker
//! keeps the motion separately for the live status stream. AJAR carries no
//! direction: a closure that was last seen closed is opening, and one that
//! was last seen open is closing. Nothing here is persisted.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;

use crate::models::telemetry::{ClosureTransition, TelemetryEvent};

/// Motion that never settles (for example a missed final frame) stops
/// showing after this long.
const MOTION_TTL: Duration = Duration::seconds(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ClosureMotion {
    Opening,
    Closing,
}

pub type ClosureMotionMap = BTreeMap<String, ClosureMotion>;

#[derive(Debug, Default)]
pub struct ClosureMotionTracker {
    settled_closed: BTreeMap<String, bool>,
    motion: BTreeMap<String, (ClosureMotion, DateTime<Utc>)>,
}

#[derive(Clone, Copy)]
enum ClosureObservation {
    Settled(bool),
    Moving(ClosureTransition),
}

impl ClosureMotionTracker {
    /// Returns a replacement map only when visible directions change.
    pub fn observe(
        &mut self,
        event: &TelemetryEvent,
        now: DateTime<Utc>,
    ) -> Option<ClosureMotionMap> {
        let readings = closure_values(event)
            .into_iter()
            .filter_map(|(field, closed)| {
                closed.map(|closed| (field, ClosureObservation::Settled(closed)))
            });
        let transitions = event.closure_transitions.iter().flat_map(|values| {
            values.iter().map(|(field, transition)| {
                (field.as_str(), ClosureObservation::Moving(*transition))
            })
        });
        // A transition overrides the binary reading of that same closure.
        let observations: BTreeMap<_, _> = readings.chain(transitions).collect();
        let mut changed = false;
        for (field, observation) in observations {
            let previous = self.motion.get(field).map(|(motion, _)| *motion);
            match observation {
                ClosureObservation::Settled(closed) => {
                    self.settled_closed.insert(field.to_owned(), closed);
                    if matches!(
                        (previous, closed),
                        (Some(ClosureMotion::Opening), false)
                            | (Some(ClosureMotion::Closing), true)
                    ) {
                        self.motion.remove(field);
                    }
                }
                ClosureObservation::Moving(transition) => {
                    let direction = match transition {
                        ClosureTransition::Opening => Some(ClosureMotion::Opening),
                        ClosureTransition::Closing => Some(ClosureMotion::Closing),
                        ClosureTransition::Ajar => match self.settled_closed.get(field) {
                            Some(true) => Some(ClosureMotion::Opening),
                            Some(false) => Some(ClosureMotion::Closing),
                            None => previous,
                        },
                    };
                    if let Some(direction) = direction {
                        self.motion.insert(field.to_owned(), (direction, now));
                    }
                }
            }
            changed |= previous != self.motion.get(field).map(|(motion, _)| *motion);
        }
        let before_expiry = self.motion.len();
        self.motion.retain(|_, (_, at)| now - *at < MOTION_TTL);
        changed |= before_expiry != self.motion.len();
        changed.then(|| self.current())
    }

    fn current(&self) -> ClosureMotionMap {
        self.motion
            .iter()
            .map(|(field, (motion, _))| (field.clone(), *motion))
            .collect()
    }
}

/// Every closure field that can be in motion, keyed as Parallax reports it.
fn closure_values(event: &TelemetryEvent) -> [(&'static str, Option<bool>); 14] {
    [
        ("door_front_left_closed", event.door_front_left_closed),
        ("door_front_right_closed", event.door_front_right_closed),
        ("door_rear_left_closed", event.door_rear_left_closed),
        ("door_rear_right_closed", event.door_rear_right_closed),
        ("closure_frunk_closed", event.closure_frunk_closed),
        ("closure_tailgate_closed", event.closure_tailgate_closed),
        ("closure_liftgate_closed", event.closure_liftgate_closed),
        ("tonneau_closed", event.tonneau_closed),
        ("side_bin_left_closed", event.side_bin_left_closed),
        ("side_bin_right_closed", event.side_bin_right_closed),
        ("window_fl_closed", event.window_fl_closed),
        ("window_fr_closed", event.window_fr_closed),
        ("window_rl_closed", event.window_rl_closed),
        ("window_rr_closed", event.window_rr_closed),
    ]
}

#[cfg(test)]
#[path = "closure_motion_contract_tests.rs"]
mod closure_motion_contract_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn at(seconds: i64) -> DateTime<Utc> {
        "2026-10-01T02:15:00Z".parse::<DateTime<Utc>>().unwrap() + Duration::seconds(seconds)
    }

    fn frunk(closed: bool, transition: Option<ClosureTransition>) -> TelemetryEvent {
        let mut event = TelemetryEvent::empty(Uuid::nil(), at(0));
        event.closure_frunk_closed = Some(closed);
        if let Some(transition) = transition {
            event.closure_transitions = Some(BTreeMap::from([(
                "closure_frunk_closed".to_owned(),
                transition,
            )]));
        }
        event
    }

    fn only(field: &str, motion: ClosureMotion) -> Option<ClosureMotionMap> {
        Some(BTreeMap::from([(field.to_owned(), motion)]))
    }

    #[test]
    fn ajar_after_open_is_closing_until_it_latches() {
        // Observed R1S frunk close: open, AJAR, then CLOSE.
        let mut tracker = ClosureMotionTracker::default();
        assert_eq!(tracker.observe(&frunk(false, None), at(0)), None);
        assert_eq!(
            tracker.observe(&frunk(false, Some(ClosureTransition::Ajar)), at(1)),
            only("closure_frunk_closed", ClosureMotion::Closing)
        );
        assert_eq!(
            tracker.observe(&frunk(true, None), at(2)),
            Some(BTreeMap::new())
        );
    }

    #[test]
    fn ajar_after_closed_is_opening() {
        let mut tracker = ClosureMotionTracker::default();
        tracker.observe(&frunk(true, None), at(0));
        assert_eq!(
            tracker.observe(&frunk(false, Some(ClosureTransition::Ajar)), at(1)),
            only("closure_frunk_closed", ClosureMotion::Opening)
        );
        // A lagging legacy "closed" does not cancel the opening.
        assert_eq!(tracker.observe(&frunk(true, None), at(2)), None);
        assert_eq!(
            tracker.observe(&frunk(false, None), at(3)),
            Some(BTreeMap::new())
        );
    }

    #[test]
    fn explicit_opening_and_closing_need_no_history() {
        let mut tracker = ClosureMotionTracker::default();
        assert_eq!(
            tracker.observe(&frunk(false, Some(ClosureTransition::Closing)), at(0)),
            only("closure_frunk_closed", ClosureMotion::Closing)
        );
        let mut fresh = ClosureMotionTracker::default();
        assert_eq!(
            fresh.observe(&frunk(false, Some(ClosureTransition::Opening)), at(0)),
            only("closure_frunk_closed", ClosureMotion::Opening)
        );
    }

    #[test]
    fn ajar_without_history_shows_nothing() {
        let mut tracker = ClosureMotionTracker::default();
        assert_eq!(
            tracker.observe(&frunk(false, Some(ClosureTransition::Ajar)), at(0)),
            None
        );
    }

    #[test]
    fn motion_that_never_settles_expires() {
        let mut tracker = ClosureMotionTracker::default();
        tracker.observe(&frunk(false, Some(ClosureTransition::Closing)), at(0));
        let unrelated = TelemetryEvent::empty(Uuid::nil(), at(31));
        assert_eq!(tracker.observe(&unrelated, at(31)), Some(BTreeMap::new()));
    }
}
