---
title: Energy view query workflows
---

# Energy view query workflows

The battery, efficiency, and idle-drain routes retain their HTTP adapters and
vehicle authorization checks. Private workflow modules now select evidence,
construct queries, and project that evidence into the existing response types.
This is a scoped replacement of these reporting workflows, not a complete
application rewrite. The project remains GPL-3.0-only; existing attribution in
`NOTICE.md` remains applicable.

## Battery evidence and projections

`routes/battery/workflow.rs` represents a requested history window explicitly.
An exhaustive signal enum selects the SOC, remaining-range, or capacity column;
request text cannot become a SQL identifier. Capacity and mileage views share
selection of qualifying capacity observations. Degradation is projected from
those typed observations instead of calculating its response fields inside SQL.
Mileage still calls the shared strict range-normalization and full-charge-range
helpers.

A capacity reference records whether its value came from stored vehicle capacity
or the maximum historical telemetry reading. Stored capacity wins only above
10,000 Wh. Historical fallback remains independent of the requested time window.
Health loads the reference, daily capacity peaks, and qualifying charge totals in
that order, then projects the original nulls, units, health cap, cycle floor, and
charging efficiency. The last-30-days health query retains its original lack of
an upper timestamp bound; future-dated synthetic observations characterize that
existing behavior. It does not use the supplied history bounds.

The phantom-drain chart folds validated periods into typed daily totals. Days are
ascending; total loss and parked hours retain the original floating-point
accumulation order and zero-duration behavior.

## Efficiency cohort planning

`routes/efficiency/workflow.rs` builds a shared vehicle/time/tag cohort, then
selects the eligibility, aggregate projection, and ordering for one of six typed
views. The shared tag parser, vehicle tag validation, and SQL tag predicate remain
unchanged. Tag validation still follows vehicle authorization and precedes the
reporting query.

The database retains weighted sums, percentiles, Fahrenheit-derived temperature
bins, tag grouping, and their established ordering. Summary and tag coverage use
the same measured-mile denominator as before. Range-versus-temperature retains
its strict greater-than-one-mile threshold and descending 500-trip limit. A
rolling accumulator owns the trend's queue and distance-weighted totals; a sample
exactly 24 hours old remains included.

## Parked history planning

`routes/idle_drain/workflow.rs` reads completed trip and charge intervals in
start/end order, merges overlapping or touching activity, and emits intervening
gaps. This replaces the former chain of ranking, grouping, merging, and lag SQL
windows. The existing 48-hour activity search padding remains in the database
selection.

The generated gap boundaries are bound as timestamp arrays to the private
evidence query. SQL applies the original requested-start and minimum-duration
conditions and reads nearest SOC/range anchors, latest eligible capacity,
telemetry counts, movement indicators, trip/charge overlap, and state coverage.
The activity query and evidence query use one read-only, repeatable-read
transaction so an intervening telemetry or activity insert cannot split their
view of history.

Existing validation calculations and their ordered exclusion reasons remain.
Charge overlap wins over trip overlap, net gain, movement, unreliable anchors,
and insufficient telemetry. Filtering excluded periods precedes the requested
limit. The chart-facing helper keeps its existing signature, quarter-hour
minimum, validated-only selection, and 500-period limit.

## Frozen compatibility evidence

Tests and complete fixtures are under `routes/__tests__/energy_*`. They were
captured and verified against the unchanged implementation before replacement.
The HTTP suite uses the production router and authentic JWT/session extraction,
current migration coordinator, production history protection, and restricted
application roles in disposable TimescaleDB databases. Bootstrap access only
creates databases, roles, and privileged extensions; application requests never
use it.

The suite compares 285 complete responses and all fields in the nine relevant
persisted tables before and after reads and a denied history deletion. Cases
include every endpoint, owner/viewer/manager and foreign access, empty history,
stored/observed capacity, sparse telemetry, null range, negative range, long and
short gaps, ongoing charge overlap, nested/touching/duplicate/reversed activity
intervals, tag cohorts, malformed inputs, and oversized bodies. Response bodies
and stable headers are compared in full. Limiter header values are normalized;
request IDs are fixed by the synthetic caller. Persisted fields are not
normalized.

Pure fixtures cover 262 complete parked-period projections and 19 complete trend
outputs, including threshold boundaries. Pure outputs are compared as canonical
serialized JSON to avoid a second decimal-to-binary conversion changing the
fixture's floating-point representation. Existing tests remain, including the
11 original ignored authorization tests executed explicitly by a fixture runner.

These fixtures do not prove performance at production history volumes or all
possible equal-distance nearest-sample ties. No secondary tie-breaker, provider
request, schema change, UI change, or history-privilege relaxation is introduced.
