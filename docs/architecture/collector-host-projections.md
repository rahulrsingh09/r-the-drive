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
