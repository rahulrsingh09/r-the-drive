# Maintenance replay workflows

The five maintenance binaries retain their command-line arguments, output text,
database contracts, and existing authorization boundaries. This change replaces
their history scan, charge-window projection, range classification, and payload
keeper selection stages. It does not authorize a production repair or change
the history protection policy.

## Snapshot scans and replay

Trip-energy recomputation, historical trip replay, and charge location/cost
sweeps consume a database cursor in pages of 256 decoded rows. Each cursor owns
one read transaction and retains its query snapshot until the scan finishes or
is dropped. Writes still use separate pool connections and preserve the original
per-row commit boundaries. The binaries retain their five-connection pools.
The cursor is configured for a full scan rather than an early exit.

This bounds the decoded input page, not the total amount of history processed.
The trip detector still retains its active trip's track points. A long scan
retains a database snapshot and its read locks; it is not a resumable job or a
whole-run write transaction. Restart reselects history using the existing
eligibility and overlap checks. Queries without an `ORDER BY` still have no
promised processing order.

Trip detection, signal fusion, power-only provenance inference, the minimum
distance check, energy policy, location resolution, insertion, annotations, and
telemetry association rules remain existing adapters and domain contracts.
The state-period reducer also remains unchanged: the full rebuild first attempts
to delete trips and the state-period helper first attempts to delete state
periods. Both operations are denied under production history protection, even
on an empty table. No permission is granted to make those paths run.

## Charge reconstruction

The two reconstruction callers share a private SQL window projection. It marks
charge-evidence segments using the existing strict two-hour gap rule, aggregates
their metadata, and finds first/last evidence timestamps. It then joins those
endpoint rows to obtain coordinates and SOC. Ordered value arrays are no longer
constructed for each segment.

The standalone repair still selects the first non-null latitude and longitude
independently. Historical reconstruction still requires a complete coordinate
pair other than `(0, 0)`. The five-minute threshold, one-point SOC increase,
PostgreSQL duration rounding, Wh/kWh arithmetic, rate classification, matching
priority, manual overrides, reference stamping, and returned count remain intact.
The count measures distinct sessions receiving telemetry stamps, not every
session updated.

The standalone repair retains its per-vehicle transaction, shared charge-history
lock, and active/pending checkpoint checks before reconstruction. Its later
canonicalization and enrichment stages remain in the same order. Migration 0031,
checkpoint custody, pin/row locking helpers, the collector, and provider stages
are unchanged. The historical rebuild's existing internal helper has no added
checkpoint admission guard; its protected top-level delete remains the first
barrier.

## Range repair and compaction

Range analysis and mutation use one private assessment query. It selects the
same vehicle/time cohort and classifies each eligible reading as unchanged,
kilometers stored as miles, or implausible after conversion. Reporting counts
those actions; mutation applies only the latter two. Model-specific ceilings,
null and capacity filters, inclusive timestamps, arithmetic order, and the
existing zero-to-null behavior are retained.

Compaction computes semantic keys for the scoped payload rows, selects one
keeper per key, and joins the remaining rows into the bounded victim cohort.
A correctly linked payload wins; ties retain capture time and UUID ordering.
The original advisory lock, missing-identity refusal, 1–50,000 batch validation,
alias/identity repointing, delete transaction, messages, and retry behavior remain.
The identity and canonicalization libraries are not replaced.

## Characterization and limits

Tests in `apps/api/src/bin/__tests__` retain full process stdout/stderr/status and
complete rows from fourteen related tables. Losslessly compressed fixtures hold
the frozen expectations. Only generated UUIDs and clocks are normalized;
annotation ordering is made stable after UUID normalization. Raw evidence is
written outside the checkout when `MAINTENANCE_ARTIFACT_DIR` is supplied.
That evidence directory is optional; the complete frozen-value assertions always
run. Ordinary tests need no evidence environment variables. Ignored database
tests still require an explicitly configured disposable database.

The original implementation passed these expectations before replacement.
Fixtures create new disposable databases, bootstrap extensions separately, apply
current migrations including 0031 through the restricted app role, and install
unchanged production history protection. Cases include empty stores, malformed
arguments, more than one scan page, null/unit/timestamp boundaries, multiple
vehicles and memberships, repeat runs, charge pins, exact gap boundaries,
compaction contention, reference rollback, later-vehicle failure, statement
cancellation, and restart.

The binaries are database-administration processes, not HTTP endpoints accepting
an authenticated owner, manager, viewer, or administrator. Tests preserve
membership-based enrichment and vehicle isolation; they do not establish new
HTTP role authorization. No provider transport is exercised. Full protected
rebuild success, concurrent schema changes, process-kill/network-partition
behavior, and production operation are not validated.

The existing native-collector charge-repair regression remains unchanged. Its
database bootstrap requires `CREATEDB`, so it runs separately with a non-superuser
fixture login and an empty, fully migrated database template. The template
already has history protection installed for that login. Protected history
DELETE/TRUNCATE permissions remain absent; extensions are installed by the
separate bootstrap role. The one-shot template preparation is test tooling
retained with the external evidence, not a runtime installation path.

Documentation impact: documentation site update required. The integration owner
registers `architecture/rewrite-maintenance-replay` in the shared sidebar.
