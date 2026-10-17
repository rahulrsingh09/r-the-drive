---
title: Acquisition implementation replacement
description: Provider acquisition, collector projections, and companion contracts.
---

# Acquisition implementation replacement

Documentation impact: Documentation site update required.

This change revisits the acquisition implementations retained by earlier bounded
rewrites. It preserves the existing GPL-3.0-only project and attribution. It does
not establish independent or clean-room authorship. Public module paths,
entrypoint signatures, provider requests, checkpoint formats, and database schema
remain compatible.

## Acquisition and lifetime

Vehicle read adapters use a typed operation plan for enrichment, wallboxes,
completed history, live curves, and charging schedules. The plan applies the
existing one-refresh authentication workflow and the history attempt/success
clocks. Schedule-write adapters still load and validate credentials before the
fixed vehicle-write denial; they do not construct a provider mutation request.
Credential refresh selects either a concurrently refreshed bundle or a newly
refreshed bundle under the credential-row lock, then commits through one exit.

The supervisor serves commands until shutdown or channel closure, then drains
workers concurrently. Its restart schedule is explicitly 5, 10, 20, 40, 80, 160,
and 300 seconds, with the existing stable-run reset. The collector acquisition
host and durable pipeline retain their existing staged implementations and
cancellation contracts.

Socket payload classification now consumes the collector's typed field registry.
Online-only observations, sensor clocks, and closure-transition hints remain
excluded from payload classification. Charging-session root presence still wins
over vehicle-state data, including an explicit null root. Provider field rejection
counts and query rendering use the existing descriptor order and wire selections.

The companion uses explicit lease, credential, connection, and wait states.
Shutdown releases only a lease the task acquired. Standalone completion and
shutdown share an exit that aborts remaining owned tasks. Closure and lock field
names share descriptors with their canonical destinations. Typed capture records
continue to exclude raw GNSS and network payloads and preserve payload caps.

## Persistence

Acquisition row plans pair each typed value with a static column name and a merge
rule. Inserts, conditional upserts, and updates keep SQL values parameterized.
Column names and SQL expressions come from source definitions, never provider
input. The plans do not add transactions or combine formerly independent writes.
They execute on the supplied pool or transaction, preserving each caller's atomic
boundary and failure ordering.

Runtime health retains the sticky `needs_reauth` guard. Payload and heartbeat
observations update their own source and receipt clocks. Charge projections retain
nullable merge precedence, PostgreSQL peak-power ordering, rounded durations, and
the open-session guard. Trip projection, identity verification, weather enqueue,
and completion acknowledgement still share one transaction. Alias repairs execute
inside their caller's transaction.

Completed-history processing admits audit evidence before reconciliation and skips
only unchanged evidence already attached to a valid session. Retention tombstones,
canonical survivor preference, first audit-error redaction, and cost recomputation
remain intact. Live curve acquisition groups timestamps while preserving arrival
order within equal timestamps and the inclusive overlap window.

Optional reverse geocoding remains denied by private-deployment policy. Its dormant
HTTP and address-writing implementation was removed; acquisition still produces no
optional geocoding request or database access.

## Contracts and verification

Original embedded tests, separate test sources, and frozen JSON fixtures are
preserved. Added tests cover every frozen socket field, UUID advisory-lock folding,
blocked optional geocoding, and live charge null/rounding/peak/closed-session
boundaries. Configured tests use disposable TimescaleDB and Redis on an internal
Docker network, including restricted application roles, actual local HTTP, failed
writes, cancellation, and checkpoint recovery. Timescale background jobs are
disabled only in these fixtures to avoid cleanup races with disposable databases.

The per-unit review inventory records new replacements, previously replaced
reducers and drivers, macro-generated implementations, and individual small
contract-forced bodies for coordinator inspection. Literal wire envelopes,
identity accessors, enum labels, fixed initialization values, and exact floating
arithmetic are not renamed merely to change textual comparison results.

No production deployment or live-provider validation is implied. The integration
owner must register this document in the shared documentation index/sidebar and
run the integrated documentation and application acceptance gates.
