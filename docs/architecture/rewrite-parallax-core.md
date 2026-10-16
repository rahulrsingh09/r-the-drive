---
title: Parallax companion stages
description: Typed decoding, canonical forwarding, snapshot planning, and isolated subscription control.
---

# Parallax companion stages

The Parallax companion opens its own read-only Rivian subscription. It shares
validated partial telemetry and the latest canonical charge-session context with
the vehicle worker. It retains its own connection, lease, heartbeat, retry loop,
and normalized storage path. The public entrypoints and protobuf definitions
remain in `apps/api/src/parallax.rs`.

## Data path

1. `transport::connection_request` builds the fixed GraphQL WebSocket upgrade
   request from the existing decrypted session bundle. `connection_init` builds
   the client initialization message. Acknowledgement retains its absolute
   20-second deadline and Ping/Pong handling. The existing 21-topic subscription
   allowlist and query are unchanged.
2. `transport::SocketAction` classifies subscribed messages. Subscription IDs
   are checked before extracting an envelope. Invalid JSON and unrelated frames
   are ignored after acknowledgement; malformed JSON during acknowledgement
   remains an error. Close, subscription rejection, and completion retain their
   existing outcomes. The driver accepts a stream/sink so tests can exercise the
   same loop without contacting Rivian.
3. Envelope admission checks the topic allowlist before decoding Base64.
   Source timestamps accept RFC 3339 or integer milliseconds; absent or invalid
   timestamps use receipt time. Unsolicited topics never reach snapshot planning.
4. `projection::CanonicalReading` decodes a supported canonical protobuf into a
   typed variant, then validates and projects it into a partial `TelemetryEvent`.
   Projection retains ordered diagnostic notes and the distinction between
   invalid, empty, and partially recognized frames. The exported decoder
   signatures and event serialization remain unchanged.
5. `forwarding::project_and_forward` classifies the canonical outcome, applies
   the five-minute-old/thirty-second-future freshness window, and performs a
   nonblocking bounded-channel send. Both boundaries are inclusive. A full,
   closed, or absent channel does not prevent independent snapshot storage.
6. `snapshot::Snapshot` produces a typed storage plan: network, efficiency,
   mass, parked windows, charge breakdown, pack energy, curve points, remaining
   time, charger status, or cold-weather readings. It separates normalization
   and source-time association from SQL execution. `parallax.rs` applies these
   plans with the existing parameterized statements and monotonic observation
   checks, then updates collector diagnostics.

## Compatibility details

Network protobuf types omit network identity tags. Signal sentinels become
null; access-technology strings longer than sixteen bytes are excluded.
Diagnostic raw payloads remain excluded for GNSS and network topics, and
canonical diagnostic fields still pass through the existing capture filter.
The collector stores normalized values and payload hashes, not raw provider
frames. Capture policy and its existing byte cap are unchanged.

Odometer kilometers project to miles using the existing conversion factor;
tire bar readings project to PSI. Temperatures retain Celsius and the existing
physical bounds. Charging power and energy remain kW and kWh. Remaining seconds
round upward to minutes. Missing values stay missing rather than being filled
from unrelated fields. A present invalid primary charging-power value does not
select the fallback power field.

Curve plans retain original segment indices and input write order. The latest
valid power uses source time, with the later input winning equal timestamps.
Each curve point is associated with the canonical session window independently.
Session bounds are inclusive. Parallax never starts or ends a charge session;
updates require matching vehicle/session IDs, an open session, and an observation
at least as recent as the stored value. Inserts remain idempotent through the
existing conflict clauses. Writes remain sequential rather than transactional;
a later database error can leave earlier snapshot writes committed.

Empty and malformed charging schemas retain their topic-specific diagnostic
behavior. For example, an out-of-range mass yields no sample but still completes
the existing meaningful-frame bookkeeping. Unknown drive-mode frames remain
ignored. No interpretation or cost correction accompanies this replacement.

`transport::RetrySchedule` preserves delays of 2, 4, 8, 16, 32, 64, and at most
120 seconds. A routine provider TTL close resets the delay and reconnects
immediately. Heartbeats retain the 30-second interval and connected-row check.
Lease contention retains the 30-second wait. Retry waits observe shutdown;
credential loading, lease acquisition, and lease release retain their existing
database cancellation behavior.

## Verification and limits

The baseline characterization commit records 418 synthetic canonical wire
cases as complete serialized events, errors, ordered notes, and body-entry
mappings. Separate fixtures record timestamp/session boundaries, subscription
contents, acknowledgement behavior, and envelope error precedence. Persistence
fixtures cover 78 topic cases replayed twice plus eight forwarding conditions,
including every allowlisted topic, malformed/empty payloads, units, null values,
curve ordering, and independent storage after full/closed-channel forwarding.
Every selected table column is compared; generated row IDs are normalized and
database-clock timestamps are checked against the current test clock before
normalization. Provider/source timestamps and payload hashes are compared exactly.

Database tests require an explicit disposable TimescaleDB URL. Each test creates
its own database, applies the authentic migration coordinator, and uses a
restricted writer role without DELETE or TRUNCATE privileges. Tests also cover
lease ownership, heartbeat failures, stale/foreign/closed-session update guards,
and shutdown during lease contention and missing-credential retry. The database
is dropped after each successful test; no vehicle account is used.

The companion has no durable outbound queue for canonical forwarding. Full
channels can drop that handoff while companion storage continues. These tests
cannot establish real-vehicle availability, provider TLS behavior, or successful
production acquisition. Live acceptance remains unverified.

This is a bounded replacement within the existing GPL-3.0-only project. License
and NOTICE attribution remain intact.

See [Backend data flow](./backend-data-flow.md) and
[Extended vehicle telemetry](../guides/extended-vehicle-telemetry.md).
