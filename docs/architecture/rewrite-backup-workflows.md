# Backup workflow stages

The backup coordinator preserves the existing HTTP contracts, persistence schema,
recovery admission, and private deployment policy. Its internal stages separate
selection and preparation from publication and terminal cleanup. This is a bounded
replacement informed by the existing implementation; it makes no clean-room or
exclusive-authorship claim. GPL-3.0-only and the repository NOTICE remain unchanged.

## Package publication

`services/backups.rs` retains runtime readiness and session-owned advisory lock
admission. The public synchronous and detached entrypoints keep their signatures.
Both call the same run coordinator after creating the durable queued row.

`services/backups/publication.rs` prepares and validates a package, then selects its
destinations. Manual and scheduled runs use the configured local/S3 choices.
Pre-restore runs keep a local safety package and skip S3 and history mirroring.
Private history backup configuration can independently require local retention.

The publication order remains local catalog, remote upload/catalog, private history
mirror, local retention, remote retention, optional staging removal, and terminal
run update. A remote upload or retention failure retains and catalogs the local
package when it otherwise would have been temporary. Errors mark the running row
failed without overwriting its last progress percentage. Cleanup checks the local
catalog before deleting a failed package. Existing partial-progress and retry
semantics remain visible in the durable tables.

Archive generation, sanitized settings, operational-history export, manifest
contracts, recovery admission/extraction, catalog reconciliation, and workspace
ownership continue to use their existing implementations. Crypto, key custody,
private deployment policy, protected history, and restore execution are unchanged.

## Imports and administrative reads

`routes/backups/import.rs` admits the declared size and sanitized filename before
creating the staging file. It records the run, receives chunks with the existing
byte, disk-reserve, and absolute deadline checks, flushes, and validates a cloned
pinned file handle. Only validated packages publish their filename and catalog
row. Failure removes the published path and marks the durable run failed.

The HTTP adapter still checks administrator access before starting the detached
recovery operation. The operation retains admission when the HTTP waiter leaves.
Local/remote materialization, checksum comparison, restore preflight and plan
binding, capability issuance, and supervisor handoff remain guarded adapters to
the completed recovery libraries.

Settings pass through parsed schedule and destination validation before endpoint
policy, prefix normalization, and secret encryption. Persistence preserves the
original clear/replace/retain secret precedence and timezone write ordering.
Read selection is represented explicitly as a bounded page, an ID, or the latest
successful run. Query aliases, ordering, null handling, pagination arithmetic,
response fields, and existing catalog reconciliation side effects are unchanged.

## Calendar and S3 transfer

A private calendar implementation serves both the next-run presentation and the
most-recent due-run calculation. It aligns the current date to the cadence and
moves to the adjacent period only when the boundary comparison requires it.
Month-end clamping, Sunday-based weekdays, equality, ambiguous local times, and
the existing nonexistent-local-time fallback are preserved.

S3 catalog state owns the normalized prefix, continuation token, and admitted
rows. Metadata requests remain serial, preserving failure order; non-package
objects are skipped, at most 100 package rows are returned, and timestamp ties
retain listing order. Multipart state owns the upload ID and completed part
receipts. Each read is bounded to the existing 64 MiB part size; completion uses
those receipts, and returned errors attempt abort. The SDK transport policy,
credential handling, retry configuration, endpoint checks, and timeouts remain
unchanged. Download byte/disk/deadline admission still uses the existing confined
staging implementation.

## Verification

Characterization is committed before the production replacement. Tests under
`routes/__tests__/backup_workflows*` and
`services/__tests__/backup_workflows/` cover complete HTTP responses and database
rows, authorization, ordered errors, settings secrets, pagination, upload and
stream failures, downloaded bytes, restore-request conflicts, malformed preflight,
schedule boundaries, retention, restart, lock cancellation, S3 pagination, and
multipart failure/abort. Additional execution fixtures create actual packages
with PostgreSQL client tools and assert complete run/catalog records for local,
safety, remote, upload-failure, and retention-failure outcomes.

Database fixtures explicitly require a disposable test URL, create unique
installations, install extensions through privileged bootstrap, apply all current
migrations, and run the application as a restricted role with history protection.
S3 uses loopback scripted HTTP with the existing development-origin policy;
no provider account or cloud resource is used. Raw bodies and state evidence stay
outside the repository. Evidence variables only enable optional file copies; when
unset, tests use owned temporary directories. Frozen assertions always run.
Frozen expectations normalize generated IDs, clocks, and
random ciphertext identity; package bytes, checksums, sizes, statuses, and messages
remain compared. Archive creation uses independent byte/hash verification and
complete row assertions because real dumps contain installation-specific data.

Run the scoped suite with an explicitly isolated fixture and evidence directory:

```sh
cargo test --locked --offline --lib workflow_contract -- --include-ignored --test-threads=1
cargo check --locked --offline --all-targets --all-features
cargo clippy --locked --offline --all-targets --all-features -- -D warnings
```

These tests do not authorize or prove a production restore. A real Garage
deployment, live supervisor cutover, process-kill cleanup, and
arbitrary concurrent catalog/restore-request mutations are outside this bounded
verification. Multipart task cancellation retains the inherited behavior: abort
is attempted for returned errors, not guaranteed after task/process termination.
The S3 scanner retains its existing 100-package cap and caller deadline; a server
that repeats continuation tokens can still cause repeated pages before timeout.
Settings and import/catalog writes retain their original transaction boundaries.
