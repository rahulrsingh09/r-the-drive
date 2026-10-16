---
title: Derived trip and fallback charging series
description: Private evidence selection, interval estimation, temperature provenance, and recorded charge interpolation.
---

# Derived trip and fallback charging series

Three private helpers behind the trip and charging APIs delegate to typed domain
stages. Their signatures, response types, arithmetic, thresholds, source labels,
and surrounding HTTP and database workflows remain unchanged. This work continues
the existing GPL-3.0-only application and retains its attribution.

## Trip power evidence

`trips/power_series.rs` selects direct telemetry when at least two rows contain
finite power or regeneration. Each row counts once, including zero-valued readings.
Otherwise, it selects the median valid capacity using the existing Wh/kWh
normalization and percentile calculation.

A streaming `SocCursor` retains the last accepted finite SOC in the inclusive
0–100 range. Changes of at most `1e-6` keep the previous timestamp. Every larger
change advances the cursor, including changes whose resulting interval is
rejected. This replaces the intermediate SOC vector and adjacent-window traversal
without sorting or bridging rejected transitions.

`SocChange` applies the inclusive 2–300 second and 0.05–0.5 percentage-point bounds,
then evaluates the original power expression in its original arithmetic order.
Nonfinite estimates and magnitudes greater than 750 kW are rejected.
`EstimatedIntervals` preserves accepted interval order and calculates the median,
90th percentile, and coverage with the existing formulas. Coverage divides by a
trip duration of at least one second and clamps to 0–100 percent. Interval lookup
remains half-open.

## Temperature evidence and provenance

`trips/temperature_series.rs` infers overall vehicle, weather, mixed, or unavailable
provenance only when the trip has no stored source string. Stored strings,
including empty or unknown strings, remain authoritative for the response source
and attribution decision.

The timeline pairs elapsed values with their existing temperature entries,
preserving duplicates and skipping absent entries. It selects the first nearest
weather row within an inclusive 30 seconds. Raw vehicle provenance determines the
sample label but still permits the matched weather timestamp. Weather
temperatures do not replace the already merged sample temperature in this stage.
Nonfinite temperatures retain their existing JSON representation.

The elapsed-distance calculation retains its original signed integer arithmetic.
Extreme opposing `i32` elapsed values therefore still panic on overflow in debug
builds; this compatibility replacement does not correct that behavior.

## Recorded charging fallback

`charging/fallback_series.rs` separates session elapsed time from SOC projection.
One recorded point receives the original endpoint midpoint. Multiple points
interpolate between the first and last supplied timestamps without sorting,
deduplication, or SOC range validation. Both axes retain whole-second truncation
and their original minimum spans and clamps.

Recorded power defaults null to zero and clamps to the supplied cap. Sample
source, charger type, session ID, and the `recorded` method remain unchanged.
Nonfinite SOC arithmetic and power inputs preserve the existing float behavior.
Invalid negative or NaN caps still panic for nonempty input; empty input still
returns before cap validation.

## Verification

The characterization commit precedes implementation changes. Five ordinary tests
under `routes/__tests__/derived_*_series.rs` preserve 486 complete synthetic output
records and the existing panic boundaries. Floating outputs include bit
representations, and the complete serialized records are compared exactly.
Optional evidence export is unnecessary for ordinary test execution.

The original trip and charging frozen fixtures, helper regressions, and HTTP/DB
contracts remain intact. Contract execution uses the full compiled
`routes::trips::query_contracts::` and
`routes::charging::workflow_contract_tests::` prefixes, each selecting three tests.
Their authorization wrappers also execute the nine original trip and five
original charging tests in restricted disposable databases.

The external implementation report records immutable source identities,
commands, counts, full outputs, fixture role and migration observations, and
cleanup. Shared documentation registration and combined integration checks
remain with the integration owner.
