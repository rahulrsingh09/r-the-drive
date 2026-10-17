# Domain service implementation replacement

This change replaces retained domain-service implementations while keeping the
existing HTTP, database, chart, and capture contracts. GPL licensing and the
repository's existing attribution continue to apply.

## Admission and outbound requests

Heavy-read admission remains attached to database work. Beginning a read sets
repeatable-read isolation, read-only mode, and its statement deadline. Completion
owns the permit until commit finishes. The transaction and permit are held in
one lease, so completion cannot detach one from the other. Cancellation uses a separate control
connection and retains admission until rollback drains the server response.

Outbound requests resolve and validate every address before constructing a
client pinned to those addresses. Private destinations require explicit CIDRs.
Mixed public/private DNS answers, reserved addresses, metadata destinations,
credentials, fragments, and public HTTP remain rejected. Clients disable proxy
inheritance and redirects. Response collection deducts each chunk from the
remaining byte allowance before appending it.

Address search and reverse lookup share request preparation and response
handling, with separate failure policies. Deployment policy still denies
optional-provider traffic before reading settings or issuing a request.

## Capture accounting

Capture redaction traverses JSON with a work list and removes forbidden keys at
every depth. Recording remains nonblocking, with per-window dropped-event
accounting. The writer loads unknown capture counts together, applies admission
in record order, and inserts only records belonging to the current capture.
A capture receives at most one truncation marker from a writer ledger.

Capture start and expiry use database statements that combine related state
changes. Purging uses a transaction covering both diagnostics and events.
Stop keeps its existing update-then-marker failure behavior. Export paging
decodes rows incrementally, and a typed header preserves the existing JSON
field names, nulls, timestamps, and version selection.

## Charge, trip, and weather processing

Charge selection retains source priority, external-alias priority, timestamp
ordering, and the original tie behavior. Alias writes construct their update
predicates from the fields whose non-null values may replace stored evidence.
Each alias still commits separately, preserving partial progress on later
failure. Replay moves decoded payloads into the reconciliation loop without
cloning their JSON.

Charge identity status updates share one transition statement. Index admission
still distinguishes ordinary tables from hypertables. Retention reconciliation
claims bounded rows, clears only references to absent payloads, and preserves
the durable identity and alias records.

Active-tail repair computes charging evidence once per session before selecting
adjacent candidates. Merge fields are grouped by their existing precedence:
survivor values, later-session values, maximum readings, and latest observation
timestamps. Journal snapshots, pin checks, rollback guards, and transaction
boundaries still apply to every repair.

Trip and charge annotation writes share a typed patch representation. All
identifiers and columns used to construct SQL come from fixed internal choices;
data values use bind parameters. Null enrichment values cannot erase existing
annotations. Vehicle enrichment still completes geofences, addresses, and
weather in that order.

Weather samples are inserted in one batch inside the existing replacement
transaction. Weighted averages and nearest-hour selection retain floating-point
operation order, tolerance, and tie behavior.

## Demo, charts, settings, and cache

Demo address and software rows use batch insertion. Latest status is assembled
as structured data and decoded through the database row type, with timestamps
truncated to the same microsecond precision used by SQLx. Existing full-row,
floating-point, timezone, sparse-data, rollback, and role-denial fixtures remain
the compatibility oracle. Protected-history refresh remains denied; no history
privileges are added.

Chart helper validation keeps error ordering, accepted color syntax, transform
bounds, and bundled rendering rules. Timezone writes update application and
backup settings in one statement within the caller's transaction. Reads try
application settings before backup settings, discarding blank values only in
the application setting. An invalid selected timezone remains an error.

The basemap cache keeps its own bounded LRU index, byte accounting, and TTLs.
Its Lua operation evicts at most 64 candidates and removes partially written
payload metadata when a write fails. Operational Redis keys remain outside this
cache's eviction namespace.
Maintenance checks expiry and removes legacy permanent keys inside one Redis
script, so another write cannot replace a key between the check and removal.
