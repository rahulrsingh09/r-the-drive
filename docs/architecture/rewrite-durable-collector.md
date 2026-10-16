# Durable collector processing

The per-vehicle worker owns acquisition and recovery. `durable_processing.rs`,
declared by `ingestion/worker.rs`, owns the ordered processing of each inbound
envelope. It uses the existing PANDA storage contracts and R:The Drive behavior.
This is an evolution of the existing GPL implementation; existing attribution
and NOTICE remain applicable.

## Startup and ownership

The worker obtains its session advisory lock, loads and validates the trip
checkpoint, closes any stale recovered fragment at its last observation, and
commits the checkpoint before starting acquisition. It attempts at most sixteen
pending trip completions before reading credentials or connecting to Redis.
Invalid credentials, failed checkpoint recovery, and a competing collector keep
acquisition from starting.

Active charge repair and charge reconstruction precede acquisition. The canonical
session context is published before startup polls, periodic polls, Parallax and
the WebSocket collector start. State-period reconciliation finishes before the
first envelope is processed. The acquisition channels remain bounded at 256.
Task cancellation, the socket watchdog, restart backoff, and shutdown retain the
worker's existing behavior. Provider calls and private-deployment security remain
in their existing modules.

## Processing stages

1. **Intake** records capture diagnostics, accounts for socket traffic, and
   persists configured raw envelopes. Baseline and Parallax samples do not reset
   the socket watchdog or count as socket messages. Every envelope advances the
   cleanup and counter cadence; counters flush before the fiftieth envelope's
   legacy classification. Battery-cell metadata is handled before routing.
2. **Routing** sends envelopes without telemetry to legacy charge projection
   handling. A heartbeat with telemetry updates liveness only. Other envelopes
   with telemetry enter the trip stage, independently of their source label.
3. **Trip checkpoint** passes a cloned, fused sample to trip detection. Replayed
   samples bypass fusion and trip processing. The raw sample remains intact.
   The stage commits the checkpoint and completion outbox atomically before it
   returns a `CheckpointedTelemetry` value. A failed save retains state, retries
   with bounded backoff, and blocks downstream stages. Lock reacquisition checks
   the stored checkpoint before a retry can write.
4. **Charge association** accepts only checkpointed samples. Parallax never
   enters the canonical charge detector. Start, ongoing and final samples retain
   their canonical UUID association; the final context announces closure before
   downstream writes. The charge row is materialized before any legacy live
   projection from the same envelope is applied.
5. **Telemetry publication and storage** publishes the raw partial snapshot to
   Redis, updates liveness and latest status, evaluates suppression, and writes
   raw telemetry when selected. Fusion output is never persisted in place of
   raw telemetry. Same-timestamp partial updates preserve an existing trip association and
   prefer an incoming non-null charge association.
6. **Lifecycle** updates state periods and poll signals only when the envelope
   has appropriate evidence. An active canonical charge blocks Parallax power
   from changing the charging period or poll cadence without charger-state
   evidence. Capture diagnostics follow these decisions, then software and OTA
   version tracking run.
7. **Charge completion** removes the live Redis projection and persists the
   completed session. The outer worker then schedules live and historical API
   enrichment in their original order.

Maintenance ticks drain one trip completion or reconcile the bounded state tail.
Shutdown flushes counters, stops acquisition and companion tasks, and releases
the session lock. The checkpoint's serialized version and detector fields, outbox
payloads, telemetry columns, session identifiers, and public worker methods are
unchanged.

## Durability limits

Trip checkpoint and outbox writes form one transaction. Trip insertion, weather
job insertion and outbox removal form another transaction. Retry preserves user
edits to an already stored trip and rejects conflicting identities. The worker
must not consume another telemetry sample while its checkpoint save is pending.

Raw-envelope writes, Redis delivery, latest-status updates, raw telemetry writes,
software tracking and charge completion remain best effort at their existing
boundaries. In particular, the telemetry suppression gate advances its memory
before a telemetry write succeeds. A checkpoint commit is not a claim that all
later projections or raw telemetry have committed. The staged design preserves
these limits rather than implying a single transaction across services.

## Verification

The ignored `durable_collector_tests` and `trip_capture::tests` suites use an
explicit `TRIP_DURABILITY_TEST_DATABASE_URL` for a disposable TimescaleDB server.
Each test creates and migrates a uniquely named synthetic database and drops it
on success. Pipeline tests also require `DURABLE_COLLECTOR_TEST_REDIS_URL`; Redis
keys contain randomly generated synthetic vehicle IDs. The test harness never
loads the application's configured database or real provider credentials.

Coverage includes raw versus fused persistence, partial merges, restart identity,
atomic checkpoint retries, recovery fencing, state periods, envelope routing,
legacy live projection lifecycle, software versions, suppression, canonical
charge context, Parallax bypass, and startup completion bounds. Pure routing,
snapshot, lifecycle, detector and fusion tests remain available without services.

The review report records exact commands, database evidence, fixture cleanup,
and any reconstruction limits. Shared documentation-sidebar registration is an
integration responsibility for this page.
