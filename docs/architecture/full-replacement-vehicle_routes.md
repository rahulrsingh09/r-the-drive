# Vehicle route implementation replacement

The vehicle, external connection, health and live routes retain their existing
HTTP contracts. This change replaces their remaining substantial request,
projection and transport workflows while continuing to use the existing
authorization, provider policy, cryptography and history-protection services.
It does not change deployment configuration or enable outbound vehicle writes.

## Vehicle reads and mutations

`vehicles/status.rs` is the single implementation of status assembly. A typed
snapshot reads the vehicle, runtime, credential age, latest telemetry and
field-observation timestamps in the existing order. Its projection preserves
all response fields, nulls, range normalization, sensor availability and
freshness priorities. WebSocket silence, battery, range and charging thresholds
retain their inclusive boundaries.

Vehicle settings are validated before persistence. Credential refresh verifies
the discovered account vehicle before encrypting and storing tokens, rechecks
authorization under the existing transaction locks, commits, clears staging
credentials and then waits for collector readiness. Enrollment and membership
continue to use their previously introduced transaction workflows.
Member removal reads the target role and owner count from one database
snapshot before enforcing the existing last-owner denial and deleting the
membership.

Artwork lookup distinguishes missing, restoring and valid cached assets.
Restoring responses retain their local placeholder and cache headers. Repair
jobs retain the atomic database claim, three attempts and five-minute lease
recovery. Local integrity checks and outbound policy apply before a cached
asset or provider response is accepted.

Capture routes share an authorized action dispatcher. Exports emit the header
first, then advance an ID cursor in pages of 1,000 events. A database failure
while reading a page terminates the body with an error. Capture data remains
subject to the existing sanitization service.

## Live connections

The socket loop selects among token expiry, telemetry, the five-second
authorization interval and client input. Current session and membership checks
precede telemetry delivery and keepalives. The existing bounded authorization,
Redis connection, subscription, send and close operations remain in force.
The loop releases its connection permit on exit.

Live-session HTTP reads obtain the legacy Redis snapshot before the database
observations and use the existing field-specific freshness projection. A
missing snapshot remains HTTP 204; a present snapshot remains HTTP 200.

## External connections and health

Raster requests compile provider, style and coordinates before cache lookup or
outbound work. OpenFreeMap requests share bounded response reading and failure
recording; JSON URL rewriting uses an explicit work list. Provider credentials
remain restricted to their existing destination rules. Weather configuration
continues to apply the private-deployment policy.

The address-cache purge deliberately retains an existing failure behavior:
PostgreSQL deletes matching rows, but the `RETURNING 1` result cannot be decoded
as the requested 64-bit integer, so the route returns a database error.
Correcting that externally visible behavior requires a separate change.

Extended health reads one collector row and derives the collector, Parallax
and legacy charging projections from that snapshot. Their thresholds and
fields are unchanged. This gives those three projections a consistent
observation when the collector updates concurrently. Other health channels
retain their existing independent reads.
Trip readiness evaluates heartbeat age against the PostgreSQL timestamp read
with the runtime state, checkpoint and pending completions. The 60-second
boundary remains inclusive.

## Verification and fixed contracts

Original assertions and golden fixtures remain unchanged. Additional
characterization covers the complete status response, all extended-health
channels, stewardship counters, bounded HTTP body reading, capture pagination
and streaming errors. Configured tests use disposable PostgreSQL and Redis
instances on an internal network.
Backfill coverage checks both POST aliases, read versus manager access,
missing credentials, already-running responses and all service error
conversions without launching a provider request.

Fixed route declarations, wire-field projections, SQL schema predicates,
cryptographic library adapters and minimal authorization predicates have
individual dispositions in the accompanying review inventory. They are not
claimed as newly authored algorithms. The original source lineage and
GPL-3.0-only attribution remain applicable.

## Integration follow-up

The coordinator must register `architecture/full-replacement-vehicle_routes`
in `apps/docs/sidebars.ts` and link this page from `docs/index.md`. Those shared
navigation files belong to the coordinator; this leaf change intentionally
leaves their update as tracked integration work.
