# Metrics query workflows

This page describes the metric, overview, state timeline and parked-energy query
implementation. The documentation ID is `architecture/rewrite-metrics-queries`.
This is an internal architecture update. GPL-3.0-only and the repository's NOTICE
attribution continue to apply; these changes make no clean-room or exclusive
authorship claim.

## Planning and projection

The metric router keeps its HTTP extractors, public payloads, authorization order,
resource admission, deadlines and bounded response transport. Its private modules
separate three decisions:

- `metrics/catalog.rs` retains the ordered catalog definitions. An identifier index
  resolves requests, and a position map coalesces repeated batch selections while
  retaining the first requested position and unioning the latest/series flags.
- `metrics/planning.rs` selects a typed source relation and measure, then compiles
  the requested latest or historical span into a query plan. Each plan carries its
  SQL, scalar or weighted row projection, and whether the shared trip-tag binds
  apply. SQL fragments come from the catalog and internal recipes; request text
  never becomes a table, column or aggregate name.
- `metrics/streaming.rs` executes the admitted read in phases: latest values,
  historical cursors, transaction commit, then the final JSON delimiter. A cursor
  owns sampling progress across fetches. The document writer owns separators and
  standalone versus batch framing. The existing two-chunk channel, 64 KiB buffer,
  10,000-row fetch size, cancellation and admission cleanup remain unchanged.

The compiler consolidates the earlier independent scalar, raw, aggregate,
telemetry and filtered-trip SQL branches. The weighted efficiency calculation and
trip-tag predicate remain the shared implementations. The catalog entries,
units, labels and `is_series_metric` signature remain unchanged for chart callers.

## Compatibility details

Telemetry latest values still select the latest non-null reading across retained
history, independently of the requested range. Summary latest values retain
inclusive start bounds and their existing zero/null rules. Lifetime ignores an
explicit lower bound. The default lower bound still uses the current clock rather
than subtracting from an explicit upper bound.

Unfiltered aggregated `total_miles` still prefers available `odometer_daily`
evidence and otherwise groups trip distance by day, regardless of the requested
bucket. Unfiltered raw trip metrics retain their original row inclusion rules;
active trip-tag filters retain their different raw grouping and positive-distance
rules. Charging and telemetry metrics ignore trip cohorts as before.

Full density selects raw rows without a point cap. Compact density keeps the
existing bucket resolution and clamps the requested count to 2–96. Samples use
the same integer-spaced source positions, retaining the first and final point.
The count and cursor remain inside the admitted read transaction. Read failure
or cancellation aborts the response body; completion is published only after
commit.

The overview gathers its optional live/trip/charge tiles and today's odometer
value through a single SQL evidence statement, then decodes the existing tile
contracts and adds response clocks. This replaces the prior four-query fan-out;
concurrent ingestion is now observed within that statement's snapshot. State
timeline planning resolves bounds and the existing upper-only limit clamp,
selects the bounded period IDs, then projects their stored fields. Negative
limits still reach PostgreSQL and retain the existing error response. Parked
energy ranks evidence by source timestamp within each period window, selects the
first row and retains window ordering and every nullable energy/range field.
Tied timestamps retain no additional ordering guarantee.

## Characterization and boundaries

`routes/__tests__/metrics_queries` contains frozen complete HTTP responses,
stream bytes and persisted evidence captured on the pre-replacement code. The
DB tests create unique local databases and restricted roles, install extensions
with a separate bootstrap account, invoke the current migration coordinator and
apply production history protection. The fixtures verify denied history deletion
and unchanged rows around read and interruption sequences.

The HTTP matrix covers catalog entries, each source family, empty and sparse
history, all historical buckets, tag cohorts, duplicate selections, nullable
units, malformed requests, body/batch limits and owner/viewer/manager/foreign
vehicle access. A 20,001-point fixture checks multiple cursor fetches and output
flushes, full wire responses and compact sampling across fetch boundaries.
The boundary matrix covers bucket thresholds, aliases, density and sampling.
The tests marked ignored require the explicit disposable database URL and must
be run with `--include-ignored` or `--ignored`; merely compiling them provides no
DB evidence.

HTTP fixtures clear SQL statement caches between isolated requests on one
restricted application connection. Batches retain normal cache reuse within each
request. This isolates a retained baseline limitation: cached PostgreSQL cursor
metadata can cause mixed scalar/weighted series to abort with a missing-column
error. The replacement does not correct that behavior. Another retained race can
make a blocked singular metric query return either the observed database-timeout
500 response or the application-deadline 503 response; the test admits exactly
those two complete captured responses.

Normalization is limited to generated response clocks/current UTC date,
clock-dependent content length, request IDs and rate-limit clock/count headers.
Generated timestamps and content lengths are validated before normalization.
Metric timestamps, numbers, nulls, errors, array ordering and all retained rows
are compared. Fixtures use only synthetic local data and make no provider calls.

This scope does not replace the whole application. It leaves public routing,
security and history controls, shared math/filter services, schema, migrations,
generated UI defaults and the existing R UI unchanged. Fresh odometer materialized
view refresh privileges, equal-timestamp winner selection, concurrent ingestion
interleavings and cancellation after every possible buffer position are not
exhaustively characterized by this matrix.
