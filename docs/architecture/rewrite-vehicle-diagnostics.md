---
title: Vehicle diagnostic query workflows
description: Raw telemetry and lane query contracts, isolated characterization, and retained boundaries.
---

# Vehicle diagnostic query workflows

Scope is the read-only `GET /v1/vehicles/{id}/raw-data` and
`GET /v1/vehicles/{id}/telemetry/lanes` workflows. Vehicle membership,
enrollment, invitations, demo history, artwork, credentials, routes, public
request/response types, security policy, and schema migrations remain unchanged.
Documentation impact: Documentation site update required. Root integration owns
registration of `architecture/rewrite-vehicle-diagnostics` in the shared sidebar.

## Frozen characterization

The three ignored `diagnostic_contract_*_http_db` tests in
`apps/api/src/routes/vehicles/__tests__/diagnostics.rs` use a fresh synthetic
TimescaleDB database per test, current migrations, a restricted application role,
and production history protection. The fixture verifies the 32-entry migration
ledger, including `1000031`, and denial of telemetry deletion and charge-history
truncation. Optional provider traffic remains denied. No real providers run.

Characterization was captured from immutable base
`ea8ceab058c82b7e9487fff70229be33d77e783b` before production changes. The frozen
JSON files contain 170 complete HTTP responses and six complete initial/final
snapshots of the relevant application tables, plus migration/privilege evidence.
Every ordinary request also compares the entire before/after DB state. Requests
traverse the real Axum route, JWT/API-key extractor, membership checks, and live
SQL queries. These scoped route tests do not claim network socket, proxy, or
application-wide middleware coverage.

The sparse fixture includes null-only rows, separate pressure/status samples,
units, booleans, sparse strings, and rows immediately outside time bounds. The
dense fixture has 600 minute samples, independent raw pages, fixed and automatic
bucket resolutions, and point-limit boundaries. Complete responses retain all
fields, field coverage, ordering, spine, units, nulls, status, and every header
emitted by the scoped router. Selected raw fields filter rows; they do not narrow
the sample JSON shape.

## Running the tests

Use a unique internal Docker network with no published ports, cached TimescaleDB
and Redis images, and a fresh synthetic volume. Set
`VEHICLE_DIAGNOSTIC_DATABASE_URL` to the task-owned bootstrap endpoint
`postgres://<synthetic-bootstrap>:<synthetic-password>@db:5432/diagnostic_fixture`
and `REDIS_URL` to that network's Redis. Extension installation uses bootstrap;
migrations and route queries run with the restricted effective role. Do not point
the fixture at another installation.

```sh
SQLX_OFFLINE=true cargo test --locked --offline --lib diagnostic_contract -- --list
SQLX_OFFLINE=true cargo test --locked --offline --lib diagnostic_contract -- --include-ignored --test-threads=1
```

`VEHICLE_DIAGNOSTIC_EVIDENCE` optionally supplies an existing writable evidence
directory. `VEHICLE_DIAGNOSTIC_RUN` optionally labels those files. Both may be
absent, including when running the ignored tests. Regular `cargo test` skips
these explicit DB tests. `VEHICLE_DIAGNOSTIC_CAPTURE` is a characterization-only
switch to record an original baseline without comparing; it must be absent for
verification. Candidate runs never regenerate expected files.

## Nondeterminism and retained findings

Fixture IDs and stored creation/update timestamps are explicit. The only response
normalization is the two implicit current-time lane windows: current time is
asserted between request start/end, the default interval is asserted to be
exactly 24 hours, and any content-length header is verified against the actual
body before replacing its value. The from-only request uses a fixed resolution
and an empty vehicle. Unnormalized clock responses are optionally saved.

API-key use timestamps are checked against request start/end when changed, then
normalized only in the final key row. Full raw before/after key writes are saved
when evidence is enabled. Migration identity, checksums, success, and order are
compared; generated `installed_on` and `execution_time` are saved separately and
excluded from the deterministic ledger comparison. No fixed telemetry time,
field ordering, null, numerical value, error, or permission is normalized.

Two potentially misleading pre-existing behaviors are preserved as findings:

- `truncated` is true when the returned bucket count equals the limit, even when
  the window contains exactly that many buckets and nothing was omitted.
- `tire_pressure_samples` is `greatest(count(tire_fl_psi), count(tire_fl_status))`.
  Disjoint pressure/status observations therefore do not yield their union count.

Neither finding is corrected by this work. The characterization locks the existing
results; a correction needs a separate behavior-change decision and red/green proof.

The external task report contains commands, source identities, parity evidence,
cleanup verification, and integration limits. GPL-3.0-only licensing and existing
NOTICE attribution remain intact; this is not a clean-room authorship claim.
