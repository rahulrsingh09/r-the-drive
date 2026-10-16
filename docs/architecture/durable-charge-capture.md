# Durable charge capture

Charge collection checkpoints the detector's accepted state, independently of the optional telemetry history rows. This preserves energy, session identity and partial metadata across a collector restart even when a telemetry write was suppressed, failed, arrived late, or came from a Parallax projection that bypasses the canonical detector.

## Database format

Forward migration `0031_durable_charge_checkpoints.sql` adds two tables:

- `panda.active_charge_checkpoints` stores one versioned detector snapshot per vehicle, including Idle state and battery metadata. Its nullable active session reference pins the canonical row.
- `panda.pending_charge_completions` stores versioned completed-session payloads until the canonical session write commits. Pending references also pin their canonical rows.

The existing catalog maps this migration to version `1000031`. Earlier SQL files, descriptions, checksums, the private checkpoint migration at version `28`, and namespace mappings remain unchanged. No reset or migration-ledger adoption is needed for an installation on the previous catalog.

All floating-point snapshot and completion fields use their unsigned 64-bit IEEE-754 representations in JSON. This internal format preserves exact values through JSONB, including nonfinite optional metadata, without changing the public detector structs or HTTP response fields. Snapshot version, vehicle identity, active session identity and session ownership are validated on load. Invalid recovery state stops that collector rather than silently resetting it.

## Commit boundary

Each canonical telemetry envelope stages trip and charge transitions in arrival order. Parallax updates retain their provenance but do not mutate the charge detector. The collector then commits the trip checkpoint and completion queue, charge checkpoint and completion queue, and required canonical charge-row identity in one database transaction. Watch channels, Redis projections and ordinary telemetry storage follow the commit.

A failed checkpoint retries the same candidate before the next envelope is processed. A lost commit acknowledgment can retry the identical snapshot and outbox payload. Conditional snapshot updates reject a stale writer; reacquiring the collector lock verifies both detector checkpoints. The persisted arrival sequence does not use event timestamps, so equal-time patches and late observations retain the detector's original interpretation.

Telemetry suppression and a later telemetry-row failure do not undo a committed detector observation. This guarantees detector and completion durability for the committed input prefix. It does not make every raw telemetry row durable, or create an upstream acknowledgment/replay protocol for events received but not committed.

## Completion and maintenance

Completion delivery writes the canonical completed-session record and removes its outbox item in one transaction. A failed canonical write or acknowledgment leaves the durable completion available after restart. Startup drains up to sixteen pending charge completions before requiring credentials or Redis; normal collector maintenance continues retrying. Existing optional geocoding, annotations, cost computation and provider-history enrichment remain separate from the durable core record and may fail independently.

A transaction-scoped, per-vehicle charge lock coordinates checkpoint pins, completion delivery and maintenance. Its PostgreSQL two-key namespace is distinct from the collector's lifetime lock. Active and pending IDs cannot be deleted by a merge. Canonicalization defers pinned clusters and rechecks candidate session fields after locking; each cluster's reference changes and deletion are transactional. Bounded tail repair also defers pinned sessions. The telemetry-reconstruction CLI processes vehicles under this lock and skips reconstruction for vehicles with active or pending pins; its later canonicalization and enrichment steps retain their existing behavior.

## Startup, downtime and compatibility

An existing charge checkpoint is authoritative, including an Idle checkpoint. Old stamped telemetry cannot resurrect a completed session. Only an installation without a charge checkpoint uses the legacy telemetry reconstruction as its initial state, under the same lock as canonicalization. That one-time fallback cannot recover observations, arrival order or source labels missing from historical rows.

Checkpoints are retained through downtime. The existing detector's accepted-power clock and gap-integration rules are preserved; no samples are invented for the period when the collector was offline. A later accepted power observation may integrate over that gap, exactly as the existing uninterrupted detector would. This is model continuity, not a measurement of charging during downtime.

Stop older collectors before starting this version. Older binaries do not participate in charge checkpoints or pin locks and must not run concurrently on the upgraded database. An ordinary binary downgrade is unsupported after adoption; use a verified pre-upgrade backup in a separate compatible installation rather than deleting checkpoints, changing applied migrations or reusing stale telemetry as recovery state.

Normal full database recovery dumps include these nonsecret tables. Newer code can forward-migrate an older catalog; older code must reject a newer catalog. Keep checkpoint and outbox state together when restoring a database.

## Verification

The focused tests use explicit disposable TimescaleDB and Redis URLs and synthetic vehicles. They cover the three previously reproduced restart gaps, actual telemetry suppression, exact completed-session fields, nonfinite JSONB round trips, joint checkpoint rollback, completion-write and acknowledgment failure, restart, stale writers, canonicalization ordering and rollback, restricted application-role recording, and migration-prefix preservation.

From `apps/api`:

```bash
cargo test --lib ingestion::worker -- --ignored --test-threads=1
cargo test --lib ingestion::charge_detector
cargo test --bin repair_charge_sessions_from_telemetry -- --ignored --test-threads=1
```

Set `TRIP_DURABILITY_TEST_DATABASE_URL` and `DURABLE_COLLECTOR_TEST_REDIS_URL` to an isolated test stack. These tests must never target a live vehicle installation. The restricted-role regression verifies that history DELETE/TRUNCATE remains denied while checkpoint recording and charge completion succeed.
