# Telemetry projection and state history

The collector commits its trip and charge checkpoints before publishing telemetry, updating latest status or advancing lifecycle signals. This page describes the projections after that durable boundary. It does not change the accepted detector prefix or the charge completion outbox.

## Sparse observations

A frame is a patch, not a complete vehicle snapshot. Coalescing keeps the receiving vehicle identity, replaces the observation timestamp even for late arrivals, and overwrites only present fields. Sensor timestamps are patched independently. Closure movement is live-only and does not participate in coalescing or duplicate detection. Timestamp-only frames are empty for persistence suppression. Duplicate patches do not refresh the gate's observation timestamp.

The browser snapshot emits present fields, a location only when both coordinates exist, an online default of true, and an optional complete closure-motion map. It converts battery capacity to kWh and preserves the existing field aliases and enum strings. Nonfinite floating values become JSON null in this projection; database floats and the separate durable detector formats have different representations.

## Storage and suppression

Raw telemetry retains the incoming sparse frame. Same-time writes replace present values, retain the existing trip association, and prefer a new nonnull charge association. An unknown drive mode cannot replace a known mode.

Latest status protects its timestamped sensors with each sensor's own timestamp. A missing or older sensor timestamp cannot overwrite an existing value. Equal sensor timestamps are accepted. Other fields merge in arrival order even when the frame timestamp is older; the row timestamp is a maximum. The initial insert accepts values without sensor timestamps.

The suppression gate compares against both the last observed patch and the last selected persistence snapshot. Selection advances its baseline before the SQL write. A failed write therefore still affects later duplicate/threshold decisions; this existing limitation is characterized separately from durable trip/charge recovery. Latest-status and lifecycle stages continue when raw telemetry storage fails. No raw telemetry retry outbox is promised.

## State periods

A per-vehicle transaction lock serializes state-period rotation. The durable open period takes precedence over an in-memory cache. Matching state keeps the current period, even for an older observation. A differing state older than the active period is ignored. Equal-time changes may create a zero-duration closed period. Closing and inserting commit together; SQL failure preserves the previous open period.

Reconciliation considers only the newest qualifying telemetry row within 24 hours. It does not fuse older state fields into a sparse newer row. Unknown without an explicit power state leaves the existing period unchanged. Parallax observations without charger state cannot advance lifecycle signals while a canonical charge is active.

## Verification

`ingestion::worker::telemetry_contract_tests` freezes every patchable field, the complete browser projection, threshold decisions, nonfinite behavior and lifecycle precedence. `durable_collector_tests::telemetry_workflows` uses disposable TimescaleDB and Redis to freeze complete raw/latest rows and exercise late timestamps, actual failed SQL writes, state rollback/retry and source provenance. These tests supplement, rather than replace, the durable collector and charge checkpoint regression suites.
