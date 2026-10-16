---
title: Charging views and mutations
description: Charging query plans, curve evidence, chart aggregation, and ordered preference writes.
---

# Charging views and mutations

The charging route adapters retain their paths, aliases, request types, response
fields, body limits, membership checks, and manager checks. Their private modules
plan queries, select curve evidence, aggregate chart days, and coordinate ordered
writes. The cost engine, historical charge services, database schema, and R
interface remain unchanged. Documentation impact: internal architecture update.

## History and summaries

`HistoryPlan` resolves the history window, page size, offset, and optional local
day before querying. History still defaults to 90 days, clamps page size to
1–200, gives `per_page` precedence over `limit`, and gives an explicit offset
precedence over a page. Its historical `lifetime` behavior is retained: history
uses its ordinary window even when that flag is present. The summary, chart, and
analysis endpoints continue to use their existing lifetime-aware bounds.

A typed `SessionProjection` compiles list or detail SQL from one session evidence
projection. Energy and power are derived once through lateral expressions. The
plan retains the different telemetry counts: list counts samples explicitly
linked to the session; detail counts all vehicle telemetry within its window.
Latest live values still require an explicit session link. List rows and total
count are read in the same repeatable-read transaction, which is rolled back
after reading. Detail and its curve remain separate reads.

Summary reductions distinguish nullable sums, zero-default sums, conditional
energy sums, maxima, counts, and filtered counts. The SQL planner preserves each
measure's null and arithmetic rules; aggregation remains in PostgreSQL. Typed
summary evidence combines totals, capacity candidates, weekly rows, and network
rows for response projection. Capacity resolution still prefers a valid vehicle
capacity, then the latest valid telemetry candidate, then the maximum candidate.
The four evidence reads remain separate and ordered. A network-breakdown query
error still produces an empty breakdown; errors from the other reads propagate.

Chart aggregation groups source rows by local day, orders each group by timestamp
and session UUID, then reduces energy in that order. Negative or missing energy
contributes zero to daily totals while the individual session retains its
original value. Day, session, field, and unit ordering remain unchanged.

## Curve evidence

`CurveWindow` carries vehicle and session identity, authoritative bounds, and the
AC/DC cap. Detail first counts linked telemetry and chooses linked minute buckets
when at least two raw samples exist; otherwise it chooses the existing minute
aggregate. A shared bucket query computes previous SOC and elapsed time once,
then projects recorded or SOC-derived power with the existing 30–300 second
bounds. A detail result with fewer than two rows tries the stored curve union;
an empty union preserves the sparse primary result.

Analysis visits linked SOC-grouped samples, minute aggregates, then stored curve
points. Each telemetry stage requires both enough source evidence and enough
projected rows. Linked analysis retains its 10–300 second derivation bounds and
its recorded-power preference within each SOC group. Minute analysis retains the
30–300 second bounds. Open sessions still produce no analysis samples. Analysis
selection remains limited to completed, non-home sessions explicitly typed DC.

The stored union retains timestamp precedence: a Parallax point wins a collision
with a Rivian point. Detail preserves stored SOC; analysis uses the existing
endpoint interpolation helper. Unknown types retain the 22 kW cap; DC/DCFC retain
300 kW. Missing values, source labels, and power-method labels retain their
existing meanings. No provider is called by these workflows.

## Mutations and failure boundaries

A session mutation is prepared only after the existing manager authorization.
Preparation resolves absent/null/value `place_id`, explicit location mode, and
cost intent in the existing validation order. A typed location command selects
preservation, a user-owned saved place, clearing, or automatic matching. A cost
command is present only when a cost mode was supplied.

Execution retains separate writes: location selection/update, optional cost
mode update, cost recomputation, then response loading. A failure in a later step
can leave earlier writes committed. A missing session retains its existing
error ordering. Saved-place lookup remains scoped to the acting user's places,
including when that user is a vehicle manager.

A network mutation resolves the normalized vendor against observed vehicle
sessions before persisting the preference. Repricing selects only sessions with
automatic cost overrides and runs in the existing database result order. An
annotation failure can leave both the network preference and an earlier canonical
cost write committed. Manual session overrides survive network preference changes.
The workflow does not add a transaction or change the cost engine's idempotence.

## Compatibility evidence

The frozen tests and JSON live under `routes/__tests__`. The baseline was captured
and replayed before changing these workflows. The fixture uses a unique disposable
TimescaleDB database and restricted application role, the current migration
coordinator, and production `protect_history`. Bootstrap privileges install
extensions; they never execute application requests. The policy's existing
charge-session DELETE permission is retained for repair, while telemetry DELETE
and charge-session TRUNCATE remain denied.

The matrix contains 116 complete records, including 113 real Axum requests, and
156 pure boundary cases. It covers aliases, empty history, local-day filtering,
pagination, null energy, open sessions, sparse curves, minute telemetry, stored
curve collisions, owner/viewer/manager/foreign-user access, API-key vehicle scope,
malformed JSON, the 64 KiB body limit, repeat writes, and injected annotation
failures with their full persisted before/after state. The five existing ignored
charging authorization tests run explicitly in separate restricted fixtures.

Run from `apps/api` with the dedicated isolated fixture configured:

```sh
cargo test --locked --offline --lib charging_workflow -- --include-ignored --test-threads=1
cargo test --locked --offline --lib routes::charging::
cargo check --locked --offline --all-targets --all-features
```

`CHARGING_CONTRACT_DATABASE_URL` accepts only the synthetic `charging_fixture`
bootstrap database/account at `db` on the disposable network. Optional
`CHARGING_CONTRACT_ARTIFACT_DIR` records actual outputs while assertions always
compare frozen fixtures. Database-generated creation/update/computation clocks
and an open session's elapsed clock are normalized. Request IDs are fixed; dynamic
rate-limit remaining/reset headers are excluded. All other captured headers,
status codes, JSON values, array ordering, and persisted row fields are compared.
The full unnormalized capture is retained with the external validation evidence.

The fixtures do not establish production-scale latency or parity for arbitrary
concurrent ingestion and user edits. They do not force a failure specifically in
the optional network-breakdown summary query. GPL-3.0-only licensing and existing
NOTICE attribution continue to apply; this work makes no clean-room or
exclusive-authorship claim.
