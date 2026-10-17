# Query route composition and execution

Documentation impact: internal architecture update.

`build_router(AppState) -> Router` remains the API entry point. Route factories
are registered with a scope that determines their namespace, body budget,
identity or IP limiter, and response privacy policy. Common reads and writes
share limiter instances with uploads and v2 themes. Metadata and live reads
retain their separate budgets. The order of request ID handling, tracing,
compression, CORS, response logging, and security headers is unchanged.

Each owned endpoint family supplies a route table. The table combines disjoint
HTTP methods for the same path before creating the router. Axum rejects
overlapping methods. This matters for the charging compatibility endpoints,
which register multiple methods separately. Handler signatures and module paths
remain stable.

Vehicle requests select the vehicle before consulting membership, preserving the
distinction between missing vehicle parameters, integration scope denial, and
database authorization. Charging and trip compatibility endpoints dispatch
through typed request variants. Shared time bounds preserve the two independent
clock reads used when both defaults are absent. Charging curve analysis bounds
concurrent sample queries and retains session order. Detail and completed
analysis queries compile from one curve selection plan with distinct identity
and time-window filters.

Trip weather matching indexes elapsed times once and searches the two adjacent
timestamps for each weather row. Original sample positions break distance ties,
duplicate timestamps use the first sample, and existing temperatures take
precedence. Nonphysical elapsed values retain the original overflow behavior.

Metric responses own their producer task. Dropping the body aborts that task;
the existing admitted-read transaction and resource permit perform their
cleanup. JSON is written into the response buffer with rollback on serializer
failure. Chunk thresholds, timeout errors, cursor binding order, and response
headers retain their existing contracts.

Theme validation walks an ordered set of override sections. Tokens, series
slots, and brand paints each have a distinct key policy and share color-pair
validation. Validation order, byte limits, contrast thresholds, inherited token
fallbacks, and optimistic concurrency errors are preserved. Bundled chart and
theme documents are parsed once and remain immutable.

Catalog queries use explicit selection plans. Chart visibility and ID selection
share a query contract. Theme summaries use a join for the published revision,
and preference writes return their version directly. Place address writes use
one nullable-OSM-ID upsert; linked cost profile writes use a single statement
that preserves update-or-insert behavior and owner scoping. Typed place
projections preserve PostgreSQL nonfinite-number serialization. Trip tag writes
share validation and a prepared insert/update command.

Recovery requests share admin authorization and recovery admission. Artifact
origins distinguish rooted local files from a validated remote bucket and
prefix. Both download and staging use that resolved origin. A staging guard
exists before materialization and is released only after the durable restore
job and worker own the path. Catalog reconciliation preserves ordered,
individually committed updates; supervisor dispatch follows the preparation and
catalog-snapshot checkpoints.

The replacement preserves existing behavioral and security tests. Additional
tests cover router budgets and privacy headers, ordered validation errors,
Unicode boundaries, remote catalog/download/checksum cleanup, and restore
supervisor acceptance and rejection. Configured tests use disposable restricted
database roles and loopback HTTP servers on an internal Docker network.

Some small adapters necessarily express fixed contracts, such as exact enum
spellings, field projections, cryptographic formats, and public error envelopes.
The per-unit review inventory records those individually. The location
projection also preserves the existing SQL aliases and its characterized
populated-row decoding error; changing that behavior is a separate defect fix.
This work does not alter licensing or attribution.
