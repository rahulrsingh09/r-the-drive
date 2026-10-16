---
title: Places and billing profile workflows
description: Place mutation boundaries, billing profile resolution, address search evidence, and geocoder scheduling.
---

# Places and billing profile workflows

Documentation impact: internal architecture update.

The place, cost-profile, and Nominatim modules retain their HTTP routes, request
and response types, shared validators, database schema, and deployment policy.
Private child modules implement mutation planning, profile resolution, cache
selection, and scheduler state transitions. The R interface and generated
defaults are unchanged.

## Place mutation stages

`PlaceMutation` validates the request and identifies either a new place or an
existing place owned by the authenticated user. Validation still precedes the
ownership lookup on updates. Consequently, an invalid foreign-place update can
return a validation error before the not-found result for a valid request.

The transaction writes the address, resolves the linked billing profile, and
inserts or updates the geofence. Existing parameterized SQL adapters retain
address upsert behavior, billing normalization, default fields, and the
owner-scoped fallback from updating a profile to creating one. An OSM identifier
still identifies a shared address row; an address without an OSM identifier gets
a new row. Place profile removal detaches the link without deleting the profile.

On updates, the same transaction assigns the resulting profile, including null,
to linked sessions whose override mode is `automatic` and whose vehicle the
place owner currently manages or owns. This includes authorized shared vehicles
but excludes vehicles after that user's membership is removed or demoted to
viewer. Manual and free overrides are excluded. The shared cost service remains
the pricing authority and independently rejects profiles whose owners no longer
manage the vehicle. Editing a personal place or profile therefore cannot reprice
a revoked vehicle directly or supply its rate to a later authorized recomputation.

A successful commit produces `CommittedPlace`, containing the place ID and
sessions to reprice. Its finishing stage reprices those sessions sequentially and
then the HTTP handler reloads the place. A repricing failure retains the committed
place/profile/session-link changes and returns the existing error. Address or
geofence failure inside the transaction rolls all transaction writes back.
Deletion keeps the current owner filter and database constraint behavior;
linked place and profile deletions can return a database error.

## Profile resolution and persistence

`ProfileSelection` defines either a user-scoped ordered list or a single owned
profile. `ProfileValues` builds creation defaults or overlays supplied update
fields on the selected current row. An absent or JSON-null update field inherits
the current value, including timezone and effective dates; it does not clear it.
The resolved values pass through the existing validation functions to produce a
`ValidatedProfile` before persistence.

`ProfileMutation` selects insert or update and binds the same fields in the same
order as the original SQL. Selection and update remain separate database
operations. The workflow does not introduce an encompassing transaction, new
uniqueness rules, retries, or charge-session repricing for standalone profile
edits. Empty names, currency strings, session fees, and timezone strings retain
existing validation behavior rather than gaining new restrictions.

## Address search evidence

`SearchPlan` keeps the trimmed supplied query, lowercased cache query, requested
limit, and persistent digest key. Queries shorter than three UTF-8 bytes return
an empty response before cache or provider access.

Search resolves evidence in order: persistent Redis JSON, an unexpired in-memory
entry, then the guarded provider adapter. A valid non-array Redis value still
produces an empty response. Malformed Redis JSON falls through. Persistent keys
include the limit and never contain the address text. In-memory keys retain the
existing query-only identity and one-hour lifetime; changing the requested limit
can reuse that entry. Provider output is published to both caches before address
projection; memory eviction retains its expiration and 500-entry rules.

`SearchEvidence` projects the selected value through the existing address parser,
including null fields, string-only coordinates, and road/city fallback behavior.
No new geocoder permission or endpoint override is introduced. The current
private deployment policy rejects Nominatim before network access, regardless of
saved enabled settings. Existing cached results remain available under that
policy.

## Scheduler transitions

`Admission` separates interactive-priority yielding, waiting for the shared gate,
and reserving a slot. `QueueProgress` records total queued time and the sum of
gate waits separately; priority sleeps do not increment gate-wait accounting.
The shared gate still uses a 1,100 ms base interval and at most 15,000 ms extra.

`RateAdjustment` classifies responses before updating adaptive delay. HTTP 429
uses integer Retry-After seconds when available, otherwise starts at 500 ms extra
and doubles subsequent extra delay. Successful HTTP statuses reduce it by 200
ms; other statuses and transport errors leave it unchanged. Retry-After HTTP dates
remain unsupported. Search and reverse adapters, their different HTTP-error
responses, bounded response readers, and outbound guards are retained.

An interactive acquisition owns a scoped waiter guard. Success and cancellation
both release its count, including cancellation during a gate sleep or while waiting
for the state mutex. Active interactive work retains priority; canceled work
cannot leave background requests waiting permanently. This corrects the
characterized waiter leak. The private outbound policy still rejects Nominatim
before scheduler admission, so this is a latent scheduler correction and does
not enable geocoding traffic.

## Frozen evidence and limits

The characterization commit runs the old implementation against frozen complete
HTTP responses and all columns of nine relevant persisted tables. The suite
contains 119 state snapshots, including 118 HTTP responses and one denied history
write. It covers empty lists, create/update/delete and repeat operations, profile
null inheritance, owner/viewer/manager/disabled-user and integration-key access,
foreign IDs, linked automatic/manual/free sessions across vehicles, transaction
rollback, post-commit failure, malformed and oversized bodies, address parsing,
and memory/persistent cache selection.

Scheduler tests cover 54 outcome/header cases plus admission, gate waiting, and
cancellation. Database fixtures create a unique database and restricted login role,
use the current migration coordinator, install production history protection, and
connect through the production application pool configuration. The bootstrap
account installs extensions only and never handles application requests.
Synthetic users, vehicles, credentials, and Redis data stay in an isolated local
Docker network. No provider request is made.

Cross-run snapshots label generated UUIDs and server timestamps, and retain all
other fields and response headers. Raw evidence retains original values. Additional
unnormalized comparisons verify address rollback and repeated full-profile edits. These tests do not
claim live-provider success/error coverage through the disabled public path,
concurrent mutation race coverage, or exhaustive wall-clock scheduling behavior.
The provider adapters remain unchanged; provider-success cache publication is not
reachable through the current deployment policy.

From `apps/api`, run:

```sh
cargo test --locked --offline --lib places_profiles_contracts -- --include-ignored --test-threads=1
cargo test --locked --offline --lib --all-features
cargo check --locked --offline --all-targets --all-features
```

The first command needs the isolated `PLACES_TEST_DATABASE_URL` fixture and its
`redis` service. It refuses an unexpected bootstrap host, account, or database.
`PLACES_EVIDENCE_DIR` optionally records actual snapshots outside the source tree.
Ignored database tests are not counted as successful database validation.

GPL-3.0-only licensing and the repository NOTICE attribution remain in effect.
This workflow replacement makes no clean-room or exclusive-authorship claim and
does not represent a complete application rewrite.
