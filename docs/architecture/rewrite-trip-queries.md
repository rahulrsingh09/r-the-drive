---
title: Trip queries and tag assignments
description: Selection, telemetry projection, route reconstruction, and transactional tag assignment behind the trip API.
---

# Trip queries and tag assignments

The trip API keeps its existing HTTP routes, response types, vehicle membership
checks, integration-key scope, tag validators, and telemetry math. Private modules
below `routes/trips.rs` plan history selection and telemetry queries, assemble
route evidence, and project complete responses. `routes/trip_tags/assignment.rs`
coordinates assignment writes. This is a replacement of these workflows within
the existing GPL-3.0-only application; existing attribution remains applicable.

## History selection

`HistoryPlan` resolves the requested dates, escaped search text, and validated tag
cohort once. A selected-trip relation supplies the page and count queries, and the
map query uses the same selection. Annotation joins remain scoped to the requesting
user. Place, address, tag, and null-field projection stays compatible with the
existing API.

`Page` preserves the precedence of `per_page` over `limit`, the 1–200 clamp,
explicit offset over page-derived offset, and the returned page calculation. Rows
and count execute in one repeatable-read transaction, which is rolled back after
reading. The `data` and `items` aliases contain the same complete rows. Lifetime
selection starts at the Unix epoch; ordinary omitted bounds retain the existing
90-day behavior.

The tire-pressure timeline has a separate overlap selection. Its tag cohort
filters trip overlays; the telemetry sample array continues to include every
sample in the vehicle/time window. It intentionally does not apply history search
or pagination to that array.

## Telemetry evidence and projection

`Window` first resolves the requested trip within the authorized vehicle. Its
`Projection` chooses columns, trip linkage, buckets, predicates, ordering, and
limits before binding the vehicle and time window. SQL aggregation remains in
TimescaleDB, including null handling and `last`, `avg`, and `bool_or` semantics.

| Projection           | Evidence                                     | Bucket / limit           |
| -------------------- | -------------------------------------------- | ------------------------ |
| Short track          | Vehicle window, usable coordinates           | Raw / 5,000              |
| Medium or long track | Vehicle window, usable coordinates           | 15 seconds / 5,000       |
| Speed                | Requested or unassigned trip, non-null speed | 10 seconds               |
| Elevation            | Vehicle window, non-null altitude            | 10 seconds               |
| Power and series     | Requested or unassigned trip                 | 10 seconds               |
| Detail               | Requested or unassigned trip                 | Duration-derived / 2,000 |
| Power evidence       | Requested or unassigned trip                 | Raw                      |

The medium-track cache label remains `5 seconds` even though its query aggregates
15-second buckets. Track/elevation continue to admit samples linked to other trips
within the vehicle window; detail, speed, power, and series do not. These are
characterized compatibility behaviors.

`DetailEvidence` converts bucket rows into aligned sample columns, then projects
temperature provenance using the existing weather helpers. Cache hits enter the
same projection stage. The cached payload still contains merged weather values,
so cold and warm temperature provenance can differ exactly as before. Cache keys,
expiration, and failure tolerance remain unchanged. `Powered<T>` adds estimated
power and source fields to the typed power and series rows. Existing SOC thresholds,
capacity normalization, percentile calculations, and half-open intervals remain
unchanged.

## Route reconstruction

`RouteAssembly` records a candidate for each selected trip. Valid stored previews
are retained. Missing previews first use linked telemetry, then fall back to the
trip's vehicle/time window when fewer than two linked points are available. The
existing `services::trip_routes::build_route_preview` still validates and reduces
coordinates. Successful previews are persisted sequentially with version 1 before
projection. There is no batch transaction: a later failure can leave earlier
preview updates committed, as in the baseline. Response order follows selected
trip order, with unresolved previews represented by `missing_route_count`.

## Assignment mutation

After manager authorization, `AssignmentPlan::prepare` runs the unchanged batch
validation and vehicle membership checks in their existing order. It compiles the
mode into independent removal and addition stages:

| Mode    | Removal                            | Addition                                   |
| ------- | ---------------------------------- | ------------------------------------------ |
| Add     | None                               | Cross product, ignore existing pairs       |
| Remove  | Selected pairs                     | None                                       |
| Replace | All assignments for selected trips | Exact cross product, or none when clearing |

`commit` executes those stages inside one transaction. A failed insertion rolls
back a preceding replacement deletion. Validation and ownership counts remain
outside the write transaction. The response continues to report the number of
requested trips, including idempotent requests. Catalog CRUD, Unicode name
normalization, semantic colors, conflict mapping, and deletion cascades retain
the existing implementation.

## Characterization and validation

`routes/__tests__/trip_queries` contains frozen complete HTTP and persisted-row
fixtures captured on the unchanged implementation at
`fb9a756205547be7c5d42cf9464c8eec08dd6a06`. The suite covers 168 HTTP/state cases and
195 calculation boundaries, plus a restricted-database runner for the nine
existing ignored trip authorization tests. It includes empty series, sparse
samples, bucket boundaries, cache hits, paging, tag cohorts, personal annotations,
manager/viewer/foreign access, integration keys, malformed and oversized bodies,
assignment rollback, and protected-history deletion denial.

Database tests require an explicitly configured disposable TimescaleDB fixture
and a private Redis service. The bootstrap role installs extensions only; current
migrations and production `protect_history` run through a dedicated restricted
application role. Fixtures contain only synthetic local identities and never call
vehicle or weather providers. Generated UUIDs, database-clock metadata, request
IDs, reset headers, and transport content lengths are normalized; raw evidence is
retained outside the source tree. JSON values, array order, statuses, errors, and
all relevant persisted row fields are otherwise compared in full.

The normal test suite skips database fixtures unless explicitly enabled. No claim
is made here about a full application rewrite or exhaustive concurrency and
production-scale performance validation.
