# Hooks, API shaping, and theme runtime

Documentation impact: internal architecture update. Public hook signatures, HTTP
routes, cache keys, and visible theme values remain compatible.

## Query and mutation boundaries

`queryRuntime.ts` resolves authenticated query readiness, required identifiers,
previous-data behavior, and request dispatch. Domain hooks supply the existing
cache key and endpoint arguments, together with their freshness and polling
policy. The battery, efficiency, charging, trip, metrics, preferences, health,
and vehicle hooks use this shared boundary. Batch metrics and tag filters keep
their distinct wire and cache representations.

`cacheRuntime.ts` applies cache writes, partial updates, and invalidations as
explicit commands. Optimistic mutations cancel the affected query before
capturing a snapshot, apply the pending patch, and restore the snapshot on
failure. Charging mutations invalidate charging, batch metrics, and trips.
Chart mutations invalidate manager/effective catalogs and dashboards. Map-style
mutations share the established unit-preference key.

## Session and live data

The auth store publishes session fields through one transition path. Access
tokens remain in memory; persistence contains only user and vehicle selection
fields. Explicit replacement or clearing advances a store revision so a late
login or registration result cannot restore a cleared session. Bootstrap and
refresh retain the active selection until the owned-vehicle query validates it.
The transport's existing session-generation checks remain around asynchronous
requests, decoding, renewal, and publication.

Cookie renewal uses Web Locks when available, otherwise an IndexedDB lease.
The lease transaction commits before renewal starts, releases only its own
lease, closes its database on every outcome, and treats coordination failures
as network errors. A browser without either coordination mechanism fails closed.
The local promise queue is limited to server rendering and test environments.

`LiveVehicleConnection` owns socket handlers, timers, liveness probes, retry
backoff, renewal suspension, and browser wake events. Its lifetime is bounded
by the hook effect. Membership revocation removes that vehicle's live state,
selection, and cached responses. An expired JWT suspends reconnects while token
renewal runs; a disposed connection cannot retry or publish a late response.
Partial telemetry omits null fields, while full snapshots retain explicit nulls.
REST/live composition prefers newer defined fields without writing REST data
back into the live store.

## Wire and visual contracts

`rowProjection.ts` decodes ordered aliases, finite numbers, optional fields, and
fallback values. Existing trip and charge aggregate normalization remains on
the prior request pipeline; inherited series and summary shaping now uses the
projection boundary. Request parameter shaping retains separate trip and
efficiency semantics for tags and untagged selections.

Chart sources compile required columns before traversing rows. Source adapters
share request planning while retaining each source's missing-context result.
Known bundled charts continue to use the existing canonical renderer.

Vehicle artwork is indexed by semantic role, design mode, and placement before
selection. Packaged fallback paths and image markup remain unchanged. Protected
artwork uses authenticated blobs; restoring responses continue to display the
packaged fallback, and object URLs are revoked when their lease ends. The
first-party artwork URL predicate remains unchanged.

## Theme files

Built-in palettes and RAD vector output are fixed data. Override validation,
resolution, stable serialization, manifest indexing, and file reconciliation
are separate operations. Generation and checking share the same output plan;
checks never rewrite files. Theme hashes and all palette values retain their
existing wire format.

The baseline's theme asset check reports stale RAD artwork: the older generator
output differs from the R artwork currently shipped in `apps/web/public`.
This replacement preserves both the shipped assets and existing generator
output. Isolated generator tests verify reconciliation without overwriting the
application's assets. Updating those asset contracts requires coordinated
ownership of the public assets and generated manifests.

## Verification

Original behavioral, security, and golden tests remain unchanged. Added
`fullReplacementHooksTypesThemes` tests cover query option/argument parity,
late auth responses, wire projections, theme-manifest parity, asset indexing,
artwork priorities, and lease release, rejection, and ownership behavior.
