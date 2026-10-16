# Recovery operations

Shared package admission, catalog recovery, compatibility planning, restore
journals and workspace ownership live in the recovery operations services.
The implementation preserves PANDA/panda, R:The Drive behavior and existing
GPL/NOTICE attribution. It is informed by the existing implementation; it is
not a clean-room implementation or an exclusive-authorship claim.

## Responsibility stages

Admission composes a compressed-byte reader, the tar framing boundary, one
member-evidence inventory and manifest contract verification. The inventory
retains each member name once and provides component size, digest and database
magic evidence. The semantic verifier cannot substitute manifest claims for
bytes measured by the inventory.

Compatibility planning first captures the source and target profiles, assesses
blocking conditions in the existing serialized order, then constructs the
unchanged plan-digest material. Candidate preparation uses separate admitted,
verified and migrated types. Only a verified source can adopt a migration
ledger; a legacy ledger still requires physical baseline proof. Migration and
sanitation precede final ledger/schema verification. Planning and preparation
share the same ledger-adoption decision.

Local catalog reconciliation moves sorted discovery results through existing
row/active-run admission, confined file verification and publication. Only the
verified stage carries bytes-derived manifest, length and checksum evidence.
Journal reconciliation similarly prepares verified inputs before entering its
database transaction and records completion only after commit.

## Gzip integrity correction

Standalone admission previously accepted a complete tar archive when its
surrounding gzip stream lacked its eight-byte trailer. The regression fixture
produced `standalone admission accepted gzip trailer case 0` before the fix.
Corrupt trailers could also escape verification because tar EOF arrived first.

Admission now consumes gzip through its integrity trailer. This additional
decompression has a separate budget: the unused configured expanded-byte
allowance plus 64 KiB for tar padding. An archive with 128 KiB of trailing
expansion and no remaining content budget previously passed and now fails with
`RecoveryTooLarge`; ordinary 8 KiB padding remains accepted. Missing or corrupt
trailers fail as invalid packages.

This is an intentional correctness change, separate from parity refactoring.
Valid v1/v2/v3 formats, component checksums, database magic, migration metadata,
complete compressed-file checksums (including suffix bytes), and public
interfaces remain compatible. Malformed packages that relied on incomplete
gzip verification must be regenerated.

## Stable input and parser resource admission

The compressed reader hashes each byte as it supplies the parser. It never
rewinds for a second manifest pass. The returned length comes from consumed
bytes and must equal the opened file's length. Changes to unread bytes either
produce a complete, consistently hashed observed stream or fail validation;
the validator does not claim a filesystem snapshot at a particular instant.

A tar framing reader sits beneath the existing tar parser. It admits each
512-byte header before the parser sees it, tracks PAX size overrides across
intervening GNU metadata, and rejects sparse and other unsupported types before
their internal parsing. GNU long-name, long-link and local PAX payloads have a
64 KiB per-record ceiling, further limited by the configured member budget.
Their combined size cannot exceed `max_manifest_bytes`, and their count cannot
exceed three times `max_members`. Retained member-name bytes also have a
`max_manifest_bytes` budget. Normal file contents retain their independent
member, expanded, metadata and compression-ratio limits.

Production extraction first makes a private copy on the destination backup
volume. The copy has exclusive creation, byte/deadline/free-space checks and an
owning workspace lease. After copying, the writer closes; on Unix the copy is
read-only. Admission and extraction both use this same private file. This
prevents later changes to the original path from reaching the extractor's
unbounded tar extension handling before its final checksum comparison.
The copy stage and extraction stage each observe the configured recovery
deadline. Copying adds up to one compressed-package-sized temporary file.
Dropping an HTTP waiter does not release a blocking worker's workspace.

These are intentional fail-closed changes. Oversized extension metadata,
directory payloads, duplicate directory aliases, invalid UTF-8 names and
changing input lengths are rejected earlier. Ordinary GNU/PAX names, portable
separators, v1/v2/v3 manifests and full compressed checksums remain supported.

## Journal and catalog correctness

Restore journal reads are confined to the configured directory and reject
symlinks, non-regular files and filename/record identity mismatches. Publication
uses a unique, exclusively created private staging file, a bounded serializer,
file synchronization and atomic rename. Failed publication removes only its
own staging file. Per-job filesystem locks serialize journal mutations across
processes. Lock files persist so later contenders cannot lock different inodes
for the same job. Public full-record `write` remains a replacement operation;
callers that independently read and later write a whole record retain that
last-writer behavior. The `update` and `fail` operations hold the lock through
their read, change and publication.

A journal record is bounded by `max_history_bytes + 4 * max_manifest_bytes +
64 KiB`; this includes the catalog, plan and candidate report. The existing
public projection still omits artifact paths, capabilities and private catalog
contents. Token generation, hashing and comparison are unchanged.

Catalog snapshots use one repeatable-read transaction. A server-side size
check limits client materialization to the default operational-history budget
(16 MiB) before rows are fetched. Local filesystem scans retain sorted,
hidden-file-filtered discovery, with at most `max_members` package paths,
64 KiB per path and the configured recovery deadline.

Catalog merge now commits runs, artifacts and restore requests together.
Terminal-job reconciliation validates its artifacts before database writes,
commits history and final status in one transaction, then marks the journal
reconciled. A failed journal update after commit can safely retry the
idempotent database work. Source availability remains inert; the target's S3
locator wins and displaced historical IDs remain unavailable without breaking
their foreign keys. Backup/recovery advisory guards retain their physical
session through acquisition and release, including cancelled queries.

The journal staging regressions previously overwrote a separate synthetic file
through a predictable temporary symlink and left a temporary file after rename
failure. Both now preserve other files and clean their own staging. A catalog
merge with a final foreign-key failure previously left an earlier run row;
it now rolls back the entire merge. Oversized journals, catalogs and scans
now fail explicitly instead of allocating without these budgets.

## Integration boundaries

No crypto libraries, policy calls, schema contracts, migrations or private
deployment modules are replaced. Candidate sanitation and safe authentication
defaults remain part of preparation; backup history mirroring and local
retention protections remain in their existing call sequence.

The shared sidebar owner must register this page. Two assertions in the
inherited extractor tests also need that module's owner: invalid UTF-8 and
post-tar expansion now fail during shared admission, so their output directory
may not exist. Their final cleanup assertion must accept either absence or an
empty directory. Validation must not be weakened to satisfy an assumption
that rejected packages reach the extraction-output stage.

Filesystem calls can block below application deadlines. Cleanup remains best
effort when the filesystem refuses removal. Private snapshot stability assumes
the configured recovery volume and service account are trusted; it is not
protection against another process with the same account deliberately changing
private files. Direct internal uses of `recovery_extract::extract` must go
through the owned workspace/public backup wrappers to receive snapshot
protection.
