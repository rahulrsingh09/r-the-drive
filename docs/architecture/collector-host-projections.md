---
title: Collector host and projections
---

The collector accepts samples through the durable trip/charge checkpoint boundary before it updates optional history, caches, counters, or provider projections. Those checkpoints, completion outboxes, source provenance and canonical charge-history locks retain the contracts in [Durable charge capture](./durable-charge-capture.md) and [Telemetry workflows](./telemetry-workflows.md).

## Workflow stages

| Stage | Responsibility | Preserved boundaries |
| --- | --- | --- |
| Acquisition host | Select socket, Parallax, timers and shutdown signals; schedule socket restarts; join acquisition tasks | Recovery and bounded startup outbox drains precede credentials and Redis; producer channels hold at most 256 frames each |
| Trip completion | Resolve missing SOC from persisted history, derive metrics and route projection, admit eligible trips | Trips under 0.1 miles are acknowledged without a trip row; admitted trip, weather job and completion acknowledgement share one transaction |
| Charge projection | Fold eligible legacy samples into restart candidates; update optional live/cache projections and completed session metadata | A durable checkpoint, including an idle checkpoint, takes precedence over legacy reconstruction; the existing accepted-power reducer and pin locks remain authoritative |
| Software history | Track observed installed versions separately from state periods and OTA availability | Observations use the event timestamp; sparse frames without a version do not mutate history |
| Stewardship | Plan counters from envelope kind and source, persist optional raw frames, maintain raw retention | Synthetic control frames do not count as received payloads; malformed JSON keeps its original raw text |

The host retries completion work every 15 seconds, reconciles state periods every 15 minutes, and restarts a socket silent for 600 seconds. Unexpected socket exits use a 1, 2, 4, …, 60 second backoff; a run lasting 300 seconds resets that backoff. Graceful stop aborts and joins the socket, gives Parallax up to five seconds, then aborts and joins companions. Dropping the worker also aborts owned tasks and closes the detached collector-lock connection.

Legacy reconstruction is a compatibility path for installations without a durable charge checkpoint. It considers the preceding 12 hours of associated telemetry, chooses the most recently observed open session within 120 minutes, and preserves first/last non-null SOC, last capacity and maximum charge limit. Timestamp ties did not have a defined winner in the original SQL; the reducer uses stable session-ID ordering. Nonfinite metadata remains nonfinite, including PostgreSQL's treatment of NaN as the maximum charge limit. This fallback does not recover omitted accepted samples or arrival order; only the durable checkpoint provides those guarantees.

Live legacy charging frames cannot create a canonical session. Meaningful live data refreshes Redis and optional SQL only while a canonical session is active. A root null clears the cache; an empty partial object preserves its existing TTL. A late meaningful frame with no active session clears the cache. Chart persistence remains independent of the cache outcome. Existing API-derived source labels and values retain the established merge precedence.

## Validation

Synthetic tests capture complete trip and charge rows, pending completion acknowledgements, weather jobs, sparse and late frames, nonfinite metadata, live cache expiry and SQL failures. A frozen characterization SQL query independently checks the new legacy reconstruction reducer. Paused-clock tests exercise the actual acquisition selector, restart limits, channel closure and cancellation. Existing detector, source, checkpoint, suppression, state-period and restricted-history tests remain in place.

These tests use disposable PostgreSQL/TimescaleDB and Redis resources. They do not use vehicle credentials, contact a live vehicle provider, or deploy an application.

## Failure and restart behavior

Software version rotation reads the open rows, plans the transition, and closes/inserts them in one transaction under a separate per-vehicle transaction lock. The process cache advances only after commit. A read, close or insert failure leaves both the prior history and retry state intact. Equal-version retries and worker restart do not insert an additional open row. The existing arrival-order policy remains: a late version observation can close a period using an earlier event timestamp; this change does not reinterpret historical timestamps.

A stewardship batch clears only after a successful SQL write. A rejected SQL write retains the batch for retry; a confirmed successful flush clears it. Counter dates are the database's current day at flush time, preserving the existing behavior across midnight and session time zones. These counters remain in-memory operational metrics: process termination while SQL is unavailable can still lose an unflushed batch. An ambiguous commit acknowledgement can also cause a counter batch to be counted twice on retry. These metrics do not use the trip/charge outbox.

Raw retention uses one transaction and a nonblocking transaction advisory lock. Each raw table has its own savepoint, so a rejected delete does not prevent maintenance of the other table. Cancellation rolls back the in-flight transaction and releases its lock. Retention has a one-day minimum; a value that cannot fit the database parameter type skips cleanup instead of overflowing into a negative window. An old process can still hold a leaked session lock from the previous implementation; closing that process or its pool releases it. The new writer touches only the two raw-event tables. Existing protected-history denials stay in force.

Detector peak power is stored in the charge-rate field. The detector does not supply voltage, so its completion projection leaves existing voltage intact and stores no invented voltage for a new row. This does not rewrite old stored values. Optional annotation or cost failure does not roll back an already committed charge completion; retry keeps the same session identity and annotation key.

Shutdown while waiting for a socket restart follows the normal flush/join/unlock path. Abrupt task cancellation still relies on RAII task abortion and closing the detached collector connection. Graceful cleanup and in-memory counters are distinct from the durable checkpoint guarantees.

There is no new migration or checkpoint format in these stages. The existing durable-charge upgrade/downgrade constraints still apply. Provider calls, authentication policy, the R UI and deployment defaults are unchanged. Validation here does not enable or perform deployment.
