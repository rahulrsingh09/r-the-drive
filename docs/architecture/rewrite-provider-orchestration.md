# Provider orchestration

Documentation impact: Documentation site update required.

This boundary manages acquisition for R:The Drive. It preserves PANDA/panda data
contracts, the current R UI, and the repository's GPL/NOTICE attribution. It is a
rewrite informed by the existing implementation, not a clean-room or exclusive
authorship claim. The compatibility baseline for this assignment is
`52b5a7544290c0c68ac2b718039daafca9bca707`.

## Ownership and adapters

`supervisor.rs` owns worker task lifetime. Each vehicle has one slot, containing
its restart history and either a waiting state or a running task with its shutdown
sender. The command loop alternates between serving commands and draining tasks.
Removing a slot cancels its pending restart before waiting for shutdown. A task
that exits normally or panics is eligible for recovery. Checks run every second;
restart delays are 5, 10, 20, 40, 80, 160, then 300 seconds. A lifetime of at least
300 seconds resets the failure count. Stop waits five seconds before aborting and
joining; whole-supervisor shutdown drains workers concurrently. Command-channel
closure follows the same shutdown path.

`ws_client.rs` owns credential reload, reconnect policy, socket protocol state,
and frame routing. Connection states are initializing, awaiting acknowledgement,
subscribing, and streaming. A generic stream/sink driver lets synthetic transports
exercise the production protocol without real sockets or credentials. The live
connector still builds the fixed-origin request with the original session headers
and `graphql-transport-ws` subprotocol.

`rivian_poll.rs` retains the public adapters. Its `requests` module binds each
read operation to its fixed origin, exact query selection, and variable spelling.
Its `execution` module owns startup stages and periodic cadence. Authenticated
requests progress through current credentials, refresh, and retry; there is at
most one refresh per adapter call. Only HTTP 401 or GraphQL `UNAUTHENTICATED`
produces the typed auth marker. Ordinary errors, including auth-looking text,
do not trigger refresh. Success resets the failure count; refresh failures and
second auth failures increment it. Three consecutive counted failures mark the
credential state as needing reauthentication.

`session_store.rs` owns token validation and comparison of the short-lived session
snapshot. Access and refresh tokens remain required, in that order; empty optional
session fields remain valid. Snapshot comparison uses app, user and CSRF tokens
plus creation time, preserving the existing concurrent-refresh decision. Encryption
and decryption still call the unchanged age JSON adapters. Refresh still locks the
per-vehicle credential row, reloads it, reuses an already-advanced snapshot or calls
the existing `rivian_refresh_csrf`, encrypts, and commits.

## Request coverage

All HTTP reads use the unchanged private outbound transport: fixed origins,
redirects disabled, no proxy, 30-second request timeout, and an 8 MiB JSON response
limit. Caller-supplied HTTP client policy cannot widen these restrictions. The
transport, `rivian_auth.rs`, and private security implementation are unchanged.

| Adapter operation | Fixed origin | Variables | Execution and persistence |
| --- | --- | --- | --- |
| `GetVehicleState` | Gateway | `vehicleID` | Startup baseline; wraps data as a `next` frame and uses the existing parser, then nonblocking delivery to the canonical worker. Full/closed queues retain sampled drop logging. |
| `getUserInfo` | Gateway | JSON null | Startup and public enrichment adapter; chooses the requested vehicle, updates trim/interior/features without clearing missing values. |
| `getRegisteredWallboxes` | Charging | JSON null | Startup and public wallbox adapter; skips missing IDs, accepts numeric/string power, upserts within the user account. |
| `getCompletedSessionSummaries` | Charging | JSON null | Startup, periodic and direct history calls; one response, no pagination. Full backfill delegates to the existing backfill service; an already-running backfill falls back to incremental sync. |
| `getLiveSessionHistory` | Charging | `vehicleId: ID!` | Public live-history adapters; skips missing timestamps, sorts points, and includes the five-minute overlap with the vehicle's latest stored point. |
| `GetChargingSchedule` | Gateway | `vehicleId: String!` | Startup and public adapters; preserves last-schedule selection, existing rows when absent, and default disabled when enabled is missing. |
| `getDepartureSchedules` | Gateway | `vehicleId: String!` | Retained direct adapter; upserts identified schedules without deleting absent rows. Deliberately absent from startup because the provider schema does not support this query. |
| `getOTAUpdateDetails` | Gateway | `vehicleId: String!` | Retained direct adapter; stores a returned release-notes URL without fetching it. No periodic OTA event subscription is introduced. |
| Four schedule mutation adapters | No outbound request | Existing public input types | Return the existing typed `VehicleWrite` error. Auth-loading wrapper behavior is preserved. |

Startup executes refresh, baseline, enrichment, wallboxes, history and charging
schedule in order. A failed stage does not prevent later stages. Departure and OTA
queries are not implicitly scheduled. Periodic polls use the existing poller's
power cadence, overridden to 30 seconds while actively charging. Power/charging
watch changes wake evaluation. History is due after 300 seconds in Ready/Charging
power states or 1800 seconds otherwise. The attempt clock advances before I/O,
including failed attempts; this avoids immediate retry storms.

## Frame coverage and reconnects

Initialization sends the existing client identity/version and user session token.
Before acknowledgement, text is diagnostic control data and WebSocket pings get
pongs. Acknowledgement precedes both subscriptions: ID `1` for `vehicleState`, ID
`2` for `chargingSession`. The departure subscription constant remains available
but is not sent.

Streaming data is routed by its payload root, independently of subscription ID.
A present `chargingSession` root, including null, wins over vehicle state. The
existing parser supplies telemetry and battery-cell metadata; online-only telemetry
is a heartbeat. Malformed JSON becomes a control event. Charging subscription
`error`/`complete` frames remain control events; corresponding vehicle-state frames
produce the typed schema rejection. Known fields are disabled on the second
rejection, separately for each connection loop. Four known unsupported fields stay
disabled initially; unknown names never enter the query.

Close code 4420 renews immediately only with the exact reason `Connection TTL
expired`. It resets reconnect backoff. Code 4410 publishes the no-subscriptions
diagnostic and retries; other closes and EOF retry. Stream errors continue to wait
for subsequent frames/EOF as in the baseline. Binary/pong/frame messages are ignored.
Credential-load failures use deterministic exponential delay. Socket failures add
0–25% jitter; the configured base delay is bounded, but jitter can exceed that base
cap. Ordinary connection success does not reset backoff. Each attempt reloads the
vehicle's latest encrypted credentials. No new connect or acknowledgement timeout
is introduced; worker watchdog and cancellation remain the lifetime controls.

## Reconciliation, retention and isolation

The rewrite does not alter database schemas or reconciliation SQL. Completed
history holds the existing per-vehicle transaction advisory lock across acquisition
and reconciliation; dropping its future rolls back the lock transaction. Incremental
history allows unmatched inserts within 30 days; full history allows all eligible
summaries. The existing charge-session service remains responsible for matching
provider vehicle identity, preserving telemetry windows and aliases, and canonical
session reconciliation.

Payload audit preserves durable semantic identity, canonical reuse, survivor repair,
concurrent identity convergence, and tombstones when retention has removed the JSON
evidence. Missing evidence is never recreated merely because the provider replays it.
Only unchanged evidence already linked to a live local session skips reconciliation.
Otherwise the existing session and cost services execute, with audit failure handled
as missing evidence. The ninety-day retention policy is unchanged.

HTTP and WebSocket curve acquisitions share one persistence adapter with the original
vehicle/timestamp conflict policy. Existing non-null session linkage wins, incoming
non-null power wins, and unchanged rows are not rewritten. Normalized live-session
JSON, the `vehicle:{uuid}:live_session` key, 120-second TTL, live-session enrichment,
and chart semantics remain compatible. Parser, poller, worker, detectors and models
are external dependencies of this boundary and are unchanged.

## Evidence and integration

Synthetic fixtures freeze the baseline query bodies and variables, drive the actual
socket protocol, validate error envelopes, and exercise encrypted sessions and
persistence against a disposable TimescaleDB. Retained tests cover refresh concurrency,
payload deduplication, survivor repair, retention, and supervisor restart/cancellation.
Backend all-target checks compile the real route, worker, maintenance-binary and test
callers. No test requires a real account or provider credentials.

The integration owner must register this document in the shared Docusaurus sidebar
and rerun the combined documentation gates. This branch deliberately does not edit
that shared file.

## Intentional corrections

Cancellation now encloses credential loading, socket connection, protocol I/O and
bounded-channel delivery, and interrupts periodic history work while it is pending.
Previously shutdown was selected only during established-stream reads and between
polls; a missing acknowledgement, a full delivery channel or a waiting database
operation could delay termination. Synthetic timeout regressions fail before this
change and pass with it. Dropping pending database work retains SQLx transaction
rollback and existing per-vehicle lock ownership semantics.

WebSocket diagnostics now truncate at a UTF-8 boundary within the existing
500-byte prefix limit. Previously a multibyte character crossing byte 500 could
panic while reporting a provider error. ASCII truncation and short messages are
unchanged.

The OTA decoder explicitly accepts `getOTAUpdateDetails`, matching its GraphQL
root. The original automatic camel-case spelling was `getOtaUpdateDetails`, so
successful provider responses silently left the release-notes URL unchanged.
That older spelling remains an accepted alias. Tests verify both spellings and
the resulting database update; no new fetch or polling trigger is added.
