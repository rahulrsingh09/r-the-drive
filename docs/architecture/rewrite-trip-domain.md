---
title: Trip lifecycle and closure motion
description: Trip checkpoint, start-anchor, aggregation, and live closure-motion contracts.
---

# Trip lifecycle and closure motion

R:The Drive keeps trip detection and live closure motion in two independent,
per-vehicle reducers. `apps/api/src/ingestion/trip_detector.rs` owns trip
boundaries and accumulated trip data. `closure_motion.rs` owns transient closure
directions. This implementation retains the existing PANDA/panda interfaces,
saved data shapes, GPL-3.0-only license, and attribution in `NOTICE.md`; it is
a continuation of that codebase.

## Trip observation and lifecycle

The detector first updates the last available battery, capacity, odometer, range,
and gear observations. Missing fields leave previous readings available. A typed
start decision then selects one of three paths:

| Path | Effect |
| --- | --- |
| Idle | Advance the regeneration sample clock when appropriate and retain valid parked coordinates outside gear. |
| Start | Resolve the odometer, time, and parked-location anchor; allocate a UUID; initialize trip aggregates. |
| Active | Accumulate the frame, update the motion clock, and emit completion if the closing rule applies. |

Starting requires an explicit `Drive`, `Go`, or `Ready` power state and speed
strictly greater than 2 mph. Missing power is not implicitly filled here.
`start_decision` uses the same decision as `process`, including the existing
diagnostic strings. Signal validation and joining belong to `trip_signals`,
which remains unchanged by this workstream.

Sleep closes an active trip even when that frame reports motion. Otherwise,
closure requires more than 300 **whole seconds** since the most recent moving
frame: 300.999 seconds does not close it. A closing frame contributes its
location, battery readings, power, temperatures, elevation, drive mode, and
regeneration before completion. A start frame establishes power, altitude,
location, and the regeneration time boundary, but contributes neither
temperature averages nor drive-mode votes.

## Start anchors and completion

A shift into `Drive` or `Go` remembers the preceding odometer when available,
then the shift frame's reading. Explicit power outside gear clears the anchor;
missing power leaves it intact. A shift is usable only at or before the motion
frame and no more than 900 whole seconds old.

The start odometer chooses the fresh gear reading if it does not exceed the
current reading, then the pre-motion reading if the reported odometer advanced,
then the current or last-known reading. When the chosen start is earlier than
the current odometer, the detector uses the shift time if available and a fresh
parked fix if available. It consumes those anchors once.

Completion moves accumulated data into `CompletedTripData` and constructs an
idle state retaining last battery/capacity/odometer/range observations and
applicable parked/gear memory. Completion while still in gear establishes the
next anchor at that trip's end. UUIDs remain identical from start through
completion; later trips receive new UUIDs.

## Checkpoints and energy

`TripDetectorState`, `CompletedTripData`, and `TrackPoint` retain their existing
field names, types, order, and serialization. The ingestion worker continues
to own checkpoint persistence, replay filtering, and completion transactions.
`resume_after_restart` keeps identity and accumulated data but clears
regeneration time, last altitude, gear time/odometer, and parked fix so downtime
cannot bridge those observations. `close_interrupted_trip` emits the active
trip at the supplied time and resets all detector memory except vehicle ID.

Finite regeneration readings advance a monotonic sample clock, including zero
and positive readings. Missing and nonfinite readings do not advance it.
Negative finite power is integrated using the incoming sample's power and
nonnegative elapsed milliseconds. Duplicate and backward timestamps therefore
add no regeneration energy. This is separate from the worker's reconstruction
of historical telemetry.

Distance prefers a strictly positive odometer delta, then sums GPS haversine
segments. Energy prefers SOC loss of at least one percentage point times pack
capacity, then range loss of at least one mile times historical efficiency,
then positive distance times historical efficiency. The helper retains the
existing input acceptance, including zero or negative coefficients; this
rewrite adds no new sanitization. Equal drive-mode vote counts retain the
existing unspecified tie choice.

## Closure motion

The tracker reduces one observation per closure field. A transition overrides a
binary reading in the same frame. Explicit opening/closing supplies direction;
`AJAR` derives opening from last settled closed, closing from last settled open,
or retains an existing direction when settled history is absent.

A settled reading updates history and ends only motion toward that settled
state. A delayed reading of the starting state does not cancel motion.
Transitions refresh their timestamp even when direction is unchanged. Motion
expires at 30 seconds using the supplied observation clock, not telemetry
timestamps. Unknown transition field names retain their existing support.

`observe` emits the complete sorted direction map only when visible directions
change, including an empty map after the last movement ends. Changes to settled
history or timestamp refreshes alone do not emit. Closure motion remains
transient and is not added to trip checkpoints.

## Characterization and integration

The adjacent `trip_domain_contract_tests.rs` and
`closure_motion_contract_tests.rs` use fixed UTC synthetic streams and frozen
outputs captured from base `52b5a7544290c0c68ac2b718039daafca9bca707` before
replacement. They compare complete trip outputs, intermediate serialized
detector fields, fusion outputs/diagnostics, closure maps, and closure memory.
UUIDs are normalized only for cross-run comparisons; tests independently assert
identity continuity and new-trip uniqueness. Nonfinite inputs have direct
assertions because JSON cannot represent them faithfully. Existing tests remain
in their original modules.

Capture tests are ignored by default and require an explicit output path.
Do not regenerate the frozen fixtures to make a changed implementation pass.
Behavior corrections require their own regression and an explicit account of
changed historical outputs.

Documentation integration debt: the integration owner must register
`architecture/rewrite-trip-domain` in the shared documentation sidebar and run
the combined documentation gates. Sidebar ownership is outside this workstream.
