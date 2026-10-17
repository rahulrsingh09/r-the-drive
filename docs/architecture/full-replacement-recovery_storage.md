# Recovery storage implementation replacement

This change preserves the existing recovery API, archive versions, journal fields,
protected-history policy, and source-facing module interfaces. It retains the
project's existing provenance and licensing.

Documentation impact: Documentation site update required. During integration,
the coordinator must register `architecture/full-replacement-recovery_storage` in
`apps/docs/sidebars.ts`, which is outside this assignment's file ownership.

## Archive and storage boundaries

Archive publication builds an ordered member sequence and uses one header-writing
loop. The manifest retains its historical GNU header type; other files use regular
entries, and the artwork root uses a directory entry. File modes, member ordering,
JSON encoding, compression, and explicit empty artwork directories remain stable.
An added characterization test pins the complete compressed archive digest and
its headers. Per-component checksums and manifest descriptions share a component
inventory. S3 transfers retain their existing metadata and multipart contracts.
Catalog rows are grouped by descending timestamp as they arrive. Each group retains
arrival order and duplicate keys; the object limit counts rows rather than groups.
Multipart input fills a bounded part buffer across short reads before publishing
that part and retaining its acknowledged ETag.

Artifact access continues through directory capabilities that reject traversal,
reserved names, and symlinked parents. Admission, frozen recovery inputs, extraction
limits, staging cleanup, and the bounded journal retain their ownership boundaries.
Catalog discovery checks running jobs before admitting a file and verifies the
opened file before restoring its availability.

Backup and recovery advisory locks share a session owner. A session returns to the
pool only after PostgreSQL confirms unlock. Cancellation or an uncertain lock
result closes the physical session. Retention applies an ordered sequence of
catalog actions after removal succeeds; an unsafe or unavailable local file only
marks its catalog entry unavailable. Existing database denials still govern
history mutation.

## Candidate preparation and rollback

Host and runtime restores share the same candidate-versus-preflight comparison.
It checks both the ordered transform identifiers and the final schema fingerprint
before a safety backup or activation. Candidate credentials remain isolated to a
fresh database, have no elevated role attributes or memberships, expire within
the restore budget, and lose LOGIN and their password before activation.

Credential cleanup has explicit terminate, revoke, and drop operations. A timed-out
archive transaction is terminated before revocation to avoid waiting on a catalog
lock held by archive code. Database removal and orphan-role cleanup both run before
reporting their result. Existing activation and rollback checkpoints remain intact.

Artwork replacement and rollback use ordered filesystem actions. Directory copying
uses an explicit work list, following a link supplied as the root while skipping
nested links, matching the previous traversal contract. Process and health polling
share bounded retry logic that retains the final failure pause and last error.

## Schema proof and catalog replay

Schema fingerprinting keeps the same catalog projections and database ordering,
collects the ordered items in Rust, and streams canonical JSON into the digest.
Runtime profiles use typed database rows. Catalog snapshots preserve the read-only,
repeatable-read transaction and the pre-materialization history budget.

The scratch-schema proof worker now receives cancellation from its waiter. The
worker can abandon verification and proceed directly to its independently bounded
cleanup. Cleanup still disables only scratch-database admissions, retries only
busy-database errors, and preserves permission errors. This addresses an observed
failure of the unchanged cancellation regression test.

History replay retains bounded parsing, dependency order, per-record acknowledgments,
target precedence, and independent artifact verification. Settings restoration
continues through psql's transaction boundary; a materialized settings record is
shared by the sanitized settings and timezone writes.

## Validation

The scoped checks cover deterministic archive bytes, malformed archives and limits,
path custody, journal cancellation, S3 HTTP requests and multipart failures,
restricted-role backup HTTP behavior, schema proof faults, and actual host/runtime
restore activation, rollback, deadline cleanup, and worker restart with nonempty
history. Original assertions and goldens are retained. Windows discovery and process
branches require validation on a Windows host; Linux checks do not establish that
platform's execution behavior.
