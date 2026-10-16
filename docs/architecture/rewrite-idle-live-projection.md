---
title: Idle and live session projections
---

This change replaces the private idle-drain and live-session projection internals.
It preserves their route adapters, database reads, authorization, Redis access,
public types, and serialized responses. It builds on the existing GPL-3.0-only
implementation and retains the project's attribution.

## Parked-period projection

`routes/idle_drain/projection.rs` separates activity evidence, finite SOC
anchors, state coverage, energy conversion, and range plausibility.
`derive_phantom_period` assembles these results into the existing `PhantomPeriod`.
Excluded periods retain their calculated measurements.

Activity exclusions preserve this precedence: charge overlap, trip overlap,
SOC gain, movement. SOC validation then checks boundary freshness, battery
sample count, interior support, and contradiction with the observed SOC window.
The existing duration-dependent thresholds remain in `idle_drain.rs`.

Range projection first selects qualifying range and positive SOC anchors.
It computes each endpoint's range per full SOC, then applies the existing
average-efficiency, efficiency-drift, and expected-loss checks in order.
The 35% drift comparison and the three-times-expected-loss comparison remain
strict greater-than checks. The expected-loss floor remains five miles.
Calculations retain their original operation order and units.

Duration clamps to zero before drain and coverage calculations. A capacity
must be finite and strictly greater than 10,000 Wh to produce energy in kWh.
Rate calculations require a positive duration. State coverage below 50%
hides sleep share without excluding the period.

## Live field projection

`routes/live/projection.rs` parses an object-shaped legacy snapshot and selects
five typed observations: power, total energy, pack energy, thermal energy,
and remaining minutes. Fresh, present observations replace their corresponding
legacy fields independently. The freshness boundary includes observations
exactly 120 seconds old and accepts future observations, as before.

The selected field and its provenance are applied together. Legacy provenance
is generated only for non-null power, total energy, and remaining minutes.
Unrelated legacy properties survive. Existing provenance survives when no
new provenance entries are produced; otherwise the generated map replaces it.
An active database session is required even when a legacy snapshot exists.

No additional numeric validation is introduced: fresh nonfinite Parallax
numbers still serialize as null with Parallax provenance, negative values remain
observable, and nonnumeric legacy values retain the original behavior.

## Compatibility evidence

The unchanged source at `ea8ceab058c82b7e9487fff70229be33d77e783b` was used
to capture the added fixtures before implementation changes. Characterization
is a separate commit. Existing energy fixtures and original regressions remain
intact.

The additional fixtures compare 301 complete parked-period projections with
floating-point bit patterns, 195 complete live merges and response-adapter
outputs, and 28 routed live responses with every header and full body.
The routed fixture also compares all columns of relevant persisted rows
before and after each request and verifies Redis content is unchanged.
Only rate-limit header values use the existing fixture normalization.

Database checks use a new disposable TimescaleDB installation on an internal
Docker network with no published ports. Restricted application roles run the
authentic migration coordinator and history protection. The fixture verifies
32 ledger entries, including 1000031, and denied history privileges.
Ordinary pure tests require no evidence-export environment variable.
