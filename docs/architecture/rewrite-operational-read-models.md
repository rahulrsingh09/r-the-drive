# Operational read models

This bounded change replaces query planning and aggregation in Grafana, vehicle
health, stewardship, and ingestion capture reads. Routes, response fields, private
security, ingestion writers, and durable checkpoint policies remain unchanged.
Documentation impact: architecture documentation update required.

## Query stages

Grafana constructs an ordered plan before acquiring heavy-read admission. Each
selection resolves a metric from the existing eight-column allowlist, resolves its
vehicle override or fallback, and checks membership in request order. Compiled SQL
comes only from that allowlist. Execution retains the admitted repeatable-read,
read-only transaction, timeout, inclusive timestamps, non-null filtering, ordering,
and the 1–10,000 row limit for each target. Duplicate targets keep their positions.
The 32-target and 64 KiB body limits and error precedence are unchanged. Liveness,
metric search, and empty annotation/tag adapters retain their existing behavior.

Health obtains the latest general, tire, and closure observations with three
independent lateral selections in one statement. Each selection retains its own
non-null predicates and timestamp ordering, so an unrelated newer observation
cannot displace an older tire or closure reading. The combined result uses typed,
nullable columns rather than a JSON round trip. Other health sources, software
history precedence, the 20-version cap, thermal counts, collector health formulas,
and the durable trip-readiness query remain unchanged. The three observation
categories now share a statement snapshot; the wider health response still reads
other sources concurrently and is not a single transactional snapshot.

Stewardship aggregates the selected counters by vehicle before joining vehicle
metadata and runtime state. Vehicles without counters still receive zero totals.
The four ordered read stages remain totals, active collectors, retained raw events,
and vehicle details. The field named `totals_24h` retains the existing calendar
predicate `day >= CURRENT_DATE - 1`, including future-dated fixture rows. Collector
freshness, retention clamping, display ordering, and JWT administrator checks remain
unchanged.

Capture reads aggregate counts and latest timestamps by event kind once, then fold
that histogram into the status and export summary. Export counts and summary now
come from the same statement. Marker kinds do not contribute to event count, but
do contribute to the latest-event timestamp and exported kind counts. Truncation
is determined by a stored truncation marker, independently of dropped-event count.
The latest capture ID remains pinned for keyset pagination. The existing 1,000-row
HTTP export pages, sanitization at export, filename rules, and streaming adapter
remain unchanged. A stream can still fail after emitting earlier pages; retry
starts a new complete download.

Status continues to synthesize an expired stop reason when needed. An export
header retains the stored stop reason, including null; this historical difference
is deliberate compatibility. Capture expiry, writer, privacy filters, event caps,
and retention implementations are outside this change.

## Characterization and validation

The characterization module is declared privately from `routes/grafana.rs`; all
new tests and frozen expectations live under `routes/__tests__/operational_*`.
It exercises the actual routers and captures complete status, headers, JSON or
NDJSON, and complete records from 19 relevant tables. The synthetic fixture covers
owner, manager, viewer, administrator, super-user, foreign-user, unauthenticated,
and vehicle-scoped API-key access; sparse and populated health data; validation
ordering; malformed and oversized requests; admission denial and retry; export
pagination, markers, null and scalar fields; storage denial; stream failure; and
cancellation while SQL is blocked.

Each run creates UUID-named disposable databases and restricted application roles,
installs extensions separately, applies current public and private migrations
including migration 0031, and enables the real history protection. Reads are
checked for exact within-run relational immutability. Cross-run normalization
replaces only generated membership UUIDs, response generation timestamps, and the
HTTP lengths that depend on those timestamps. Raw evidence is retained outside
the repository. No original or shared installation, real credentials, provider
request, or cloud resource is involved.

The original implementation passes frozen expectations before replacement. The
baseline includes an inherited Grafana timestamp decoding failure on populated
queries; correctness changes must be separate from replacement parity. Missing
collector ownership can also produce the inherited nullable-running decode error.
No repair of unrelated collector behavior is claimed.

## Boundaries and integration

The shared router/module registries, health history child, private deployment
runtime and policies, migration and SQLx metadata, charge durability, provider and
worker implementations, package configuration, R UI, GPL-3.0-only license, and
NOTICE remain unchanged. This work inspected existing source and retains simple
adapters and established contracts; it makes no clean-room or exclusive-authorship
claim.

The integration owner must register `architecture/rewrite-operational-read-models`
once in the shared documentation sidebar. No sidebar or hub edit is included here.
