# Telemetry projection and state history

The collector commits its trip and charge checkpoints before publishing telemetry, updating latest status or advancing lifecycle signals. This page describes the projections after that durable boundary. It does not change the accepted detector prefix or the charge completion outbox.

## Implementation boundaries

The private worker modules keep each policy explicit:

- `telemetry_fields` lists the 95 patchable fields, whether they are payload or sensor clocks, immediate-change policy, latest-status merge policy and browser projection mapping.
- `telemetry_values` applies typed sparse patches, detects duplicate payload and builds the browser projection. It never serializes a frame to implement coalescing, so nonfinite values retain their bits in memory.
- `telemetry_gate` selects a candidate and advances observation/persistence state separately. Activity profiles contain the existing delay, distance and numeric thresholds.
- `telemetry_storage` binds typed values and generates conflict assignments from static field policies. Column names never come from a frame. Raw and latest projections deliberately have distinct drive-mode and association policies.
- `state_history` selects lifecycle evidence, plans retain/ignore/replace, then executes a rotation under the existing transaction lock.

The collector's checkpoint transaction and charge-history pin locks remain upstream of these projections. This refactoring changes workflow implementation without changing schema, route contracts, provider classification or the UI.

## Sparse observations

A frame is a patch, not a complete vehicle snapshot. Coalescing keeps the receiving vehicle identity, replaces the observation timestamp even for late arrivals, and overwrites only present fields. Sensor timestamps are patched independently. Closure movement is live-only and does not participate in coalescing or duplicate detection. Timestamp-only frames are empty for persistence suppression. Duplicate patches do not refresh the gate's observation timestamp.

The browser snapshot emits present fields, a location only when both coordinates exist, an online default of true, and an optional complete closure-motion map. It converts battery capacity to kWh and preserves the existing field aliases and enum strings. Nonfinite floating values become JSON null in this projection; database floats and the separate durable detector formats have different representations.

## Storage and suppression

Raw telemetry retains the incoming sparse frame. Same-time writes replace present values, retain the existing trip association, and prefer a new nonnull charge association. An unknown drive mode cannot replace a known mode.

Latest status protects its timestamped sensors with each sensor's own timestamp. A missing or older sensor timestamp cannot overwrite an existing value. Equal sensor timestamps are accepted. Other fields merge in arrival order even when the frame timestamp is older; the row timestamp is a maximum. The initial insert accepts values without sensor timestamps.

The suppression gate compares against both the last accepted observation and the last successful persistence snapshot. Selecting a write does not advance either baseline. A successful SQL write advances both; a threshold-suppressed observation advances only the observation baseline. Empty and duplicate frames advance neither.

A failed raw write discards the proposed gate advance, so an identical later observation remains eligible for storage. The earlier behavior suppressed this retry even after the database recovered; the baseline characterization and a separate failing-then-passing regression document that correction. Latest-status and lifecycle stages still continue when raw telemetry storage fails, and the detector checkpoints remain committed. There is no raw telemetry retry outbox: a failed frame that is never sent again is not automatically replayed.

## State periods

A per-vehicle transaction lock serializes state-period rotation. The durable open period takes precedence over an in-memory cache. Matching state keeps the current period, even for an older observation. A differing state older than the active period is ignored. Equal-time changes may create a zero-duration closed period. Closing and inserting commit together; SQL failure preserves the previous open period.

Reconciliation considers only the newest qualifying telemetry row within 24 hours. It does not fuse older state fields into a sparse newer row. Unknown without an explicit power state leaves the existing period unchanged. Parallax observations without charger state cannot advance lifecycle signals while a canonical charge is active.

## Verification

`ingestion::worker::telemetry_contract_tests` freezes every patchable field, the complete browser projection, threshold decisions, nonfinite behavior and lifecycle precedence. `durable_collector_tests::telemetry_workflows` uses disposable TimescaleDB and Redis to freeze complete raw/latest rows and exercise late timestamps, actual failed SQL writes, state rollback/retry and source provenance. These tests supplement, rather than replace, the durable collector and charge checkpoint regression suites.
