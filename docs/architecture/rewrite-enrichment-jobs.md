# Enrichment job scheduling and reconciliation

The weather, location reconciliation, and charge-history job adapters retain their
public signatures, result structures, errors, and saved formats. This maintenance
of the existing GPL-3.0-only implementation retains the project's attribution.
Documentation impact: Documentation site update required. The integration owner
must register `architecture/rewrite-enrichment-jobs` in the shared sidebar.

## Weather jobs

`process_next_job` admits a job, runs the existing enrichment stages, and persists
an explicit completion decision. Admission first calls the existing private
interrupted-job recovery policy, then checks the actual enabled settings and the
UTC daily request count. The atomic claim uses an ordered candidate CTE with
`FOR UPDATE SKIP LOCKED`; only due pending/failed rows with fewer than five
attempts qualify. Claims increment attempts and mark the row running.

| Outcome                    | Saved status | Attempt adjustment     | Next attempt                     |
| -------------------------- | ------------ | ---------------------- | -------------------------------- |
| Complete                   | succeeded    | unchanged              | unchanged                        |
| No usable location targets | unavailable  | unchanged              | unchanged                        |
| Paused                     | pending      | refund one, floor zero | now                              |
| Busy                       | pending      | refund one, floor zero | 60 seconds                       |
| Daily budget reached       | failed       | refund one, floor zero | next database-local midnight     |
| Other failure              | failed       | unchanged              | exponential/provider retry delay |

The failure path retains activity recording before rereading the persisted
attempt count. Its delay is `60 * 2^min(attempts, 8)`, with the existing 21,600
second cap, followed by the larger provider retry delay and the existing 86,400
second final cap. Terminal results clear the job error and set completion time;
retry/defer results preserve any existing completion time. Provider success is
recorded only after a successful terminal write and only if a provider was used.
Status-write errors propagate. No compensation hides their partial effects.

The worker keeps its one-second work delay and 30-second idle/error delay.
Recovery still refunds stale running claims after 15 minutes, even when the
connection is disabled. Enqueue preserves exhausted attempt counts and succeeded
status. Missing trip IDs can enter the queue; processing then records failure.
These are retained behaviors, not corrections.

The private weather implementation still owns consent, fixed destinations,
precision, request reservation, the 200-request daily budget, payload limits,
validation, redirects and outbound access. Its recovery implementation is also
retained. Route/temperature selection, keyset traversal, sample planning,
provider decoding, weighted temperature math, and transactional estimate storage
are unchanged.

## Periodic reconciliation

One cycle executes these stages in order: trip coordinates, trip geofences,
charge coordinates, charge locations, trip addresses, and weather enqueue.
Location-stage failures are logged independently; both final stages execute and
retain their results for the existing summary log. The worker still starts a
cycle immediately and sleeps six hours after the complete pass. Cancellation
stops the current future; a restarted worker performs another idempotent pass.

Charge-coordinate repair selects the same newest 500 eligible automatic-location
sessions. A bounded candidate CTE and lateral telemetry lookup resolve the
nearest non-null, non-`(0,0)` fix within the existing inclusive 600-second window.
Each candidate becomes either an unresolved decision or a guarded repair.
Writes remain individual statements in selected order. They preserve source
coordinates with `COALESCE` and recheck automatic mode and missing coordinates.
The filled counter still counts a resolved candidate even if a concurrent edit
makes its update affect zero rows. Manual/cleared locations remain excluded.

Telemetry lookup now uses one statement snapshot for the selected batch, instead
of one lookup snapshot per session. The comparison fixtures establish parity for
stable synthetic inputs; they do not establish identical observations during
concurrent telemetry insertion, every equal-distance tie plan, or arbitrary
concurrent history mutation. The 100-address, 500-coordinate and existing keyset
limits and the stable geofence/address/cost stages remain unchanged.

## Charge-history admission and task ownership

Admission locks the vehicle row, reads its credential availability and status,
and decides whether to claim, report an existing run, or report missing
credentials. A successful claim clears previous completion/count fields and
commits running status before execution. The strict 120-minute stale predicate
and the existing application-clock fallback classification are preserved,
including their exact-boundary distinction.

An admitted task owns the pool, encrypted credential snapshot and age key until
launch. The public spawn adapter detaches its task and returns the existing
`{ok: true, status: "running"}` response. Client construction retains the
30-second timeout and fallback. Encryption/decryption, enrollment, provider
requests, charge reconciliation, completion counts, and error persistence remain
in their existing stages. Cancellation does not invent a terminal status: the
running row remains recoverable through stale admission. The ingestion worker,
provider locks, durable charge state, and private deployment runtime are outside
this change.

## Verification and evidence

The baseline commit contains all 25 pre-existing module tests and eight new
workflow tests under `services/__tests__`. Tests execute against newly created
local databases using authentic migrations and a restricted app role, followed
by production `protect_history`. The former terminal cleanup assertion expecting
trip deletion was adapted to assert SQLSTATE `42501`; its enrichment assertions
remain. History protection is never disabled. Bootstrap credentials only create
disposable roles/databases and install extensions.

Eight frozen JSON fixtures compare complete ordered job, vehicle, trip, charge,
telemetry, weather, activity/settings, annotation, credential, address and
geofence rows. Assertions additionally cover retry delays, day offsets,
attempt/refund counts, concurrent claim winners, cancellation/restart, disabled
and denied weather settings, malformed bounds/orphan jobs, failed status writes,
ordered reconciliation errors, and the 500-row boundary. Existing parser and
sensor-only tests retain actual local enrichment coverage. Internal weather
outcomes and charge fetch completion are synthetic; no real provider transport
or private-policy bypass is used.

Only processing-clock fields are normalized: `created_at`, `updated_at`,
`completed_at`, `next_attempt_at`, `last_attempt_at`, `last_success_at`,
`history_backfilled_at`, and `usage_date`. Nulls and fixed fixture timestamps in
January 2020, January 2026 and January 2100 remain exact. Relative retry schedules
and usage-day offsets are asserted separately. Fixture IDs are deterministic and
are not normalized. Raw states and per-installation migration/privilege records
are retained outside the repository in the assignment's review directory.

`JOBS_EVIDENCE` enables optional evidence files. Frozen-output assertions always
run, including when that variable is absent. Ordinary tests require neither
evidence variables nor database configuration; ignored database tests require
the explicit disposable `JOBS_DATABASE_URL`.

No live-provider, full-application rewrite, deployment, or exhaustive concurrency
claim is made. Shared sidebar registration belongs to the integration owner.
