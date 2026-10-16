---
title: External connection workflows
description: Settings resolution, catalog presentation, cache traversal, and diagnostic activity persistence.
---

# External connection workflows

The external connection routes retain their existing authorization, wire types,
validation order, private outbound policy, and encrypted-secret adapters.
Private children of `routes/external_connections.rs` resolve configuration and
presentation choices. A private child of `services/external_connections.rs`
compiles diagnostic activity events into atomic database statements.

Documentation impact: internal architecture update.

## Settings and presentation

The update route first checks the account role, connection definition, and
deployment policy. Weather forms retain their fixed endpoint restoration,
approximate-coordinate requirement, and credential restrictions. Validation,
stored-basemap loading, and encryption still precede persistence.

`UpdatePlan` resolves the persisted mode and seven endpoint/presentation fields.
Disabled forms persist the disabled mode; the legacy hosted value becomes remote.
Named remote defaults take precedence over submitted values. Other submitted
values are trimmed, with empty values becoming absent. The unchanged update SQL
retains the distinction between absent values that preserve existing columns and
absent values that clear nullable columns. Secret clearing and CIDR policy writes
remain in that guarded statement.

Basemap selection has a typed preference and one key-availability decision.
Disabled and custom modes precede remote preferences. A submitted nonblank key
or retained stored key selects CARTO for automatic preference. Clearing a pinned
CARTO key resets the preference to automatic; a simultaneously submitted key
still participates in preview selection, matching the existing contract.

Catalog presentation chooses a backup, OpenFreeMap, or configured source.
Backup enablement and endpoint come from backup settings. OpenFreeMap supplies
its own privacy, terms, and attribution. The existing response assembly retains
all fields, ordering, secret-presence flags, activity dates, and global Rivian
health projections.

## Cache and activity stages

Cache traversal consumes a Redis cursor page and applies either measurement or
purge before advancing. The existing `SCAN COUNT 500` hint is retained.
Measurement excludes content-type and max-age metadata keys and tolerates
individual length failures. Purge unlinks every matching key. Both operations
retain saturating counters, zero results on connection failure, and partial
results on later scan failure. Address purge retains all relational reference
guards and its original transaction boundary.

Activity persistence describes insert fields and conflict-update fields
separately for each event:

| Event      | Conflict effect                                                  |
| ---------- | ---------------------------------------------------------------- |
| Attempt    | Refresh attempt time; increment today's count or reset it to one |
| Success    | Refresh success time and clear the request error                 |
| Failure    | Replace the sanitized request error                              |
| Diagnostic | Refresh test time, result, and sanitized test error              |

The statement compiler uses only internal column and expression constants.
Connection IDs, diagnostic results, and messages remain bound parameters.
Each event retains its own update columns and permission requirements.
Database failures remain best effort. Counter overflow rolls back the entire
statement; concurrent attempts increment atomically. Existing test-check JSON
and unrelated activity fields remain intact.

## Retained boundaries and verification

Default registration is the existing insert-if-absent statement. Optional
disablement remains one guarded update. Rivian renewal is advisory status
calculation, with existing credential-age and observed-health queries; these
files do not own a renewal executor. Provider requests, bounded response readers,
crypto, private policies, membership/enrollment, and all history and charge
durability implementations remain unchanged.

Frozen fixtures under
`apps/api/src/routes/external_connections/__tests__` compare 697 pure decisions,
76 complete HTTP/row observations, and 31 activity, renewal, and cache observations.
The original implementation passed the frozen expectations before replacement.
Database fixtures create unique databases and restricted roles, install current
migrations including namespaced migration `1000031`, and apply production history
protection. Raw outputs remain in the external review directory; only generated
clock fields are normalized for comparisons.

The matrix includes role denials, malformed and oversized bodies, private-provider
denial, secret redaction and clearing, stale weather forms, pagination, day
rollover, concurrent increments, overflow, column-scoped permission failure,
late response failure after a committed update, retries, and pool reopening.
Optional provider network paths remain denied. Successful live weather/provider
transport, process termination during a database write, and production volume
performance are outside this fixture coverage.

Existing behavior includes installation-wide catalog visibility for authenticated
users, management restricted to admin/super-user accounts, and separate settings
commit and response-read stages. GPL-3.0-only, NOTICE, and the R interface are
unchanged. This bounded replacement makes no exclusive-authorship claim.
