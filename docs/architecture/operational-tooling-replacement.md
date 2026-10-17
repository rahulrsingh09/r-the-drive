# Operational tooling implementation boundaries

The operational scripts preserve the existing command names, arguments, exit behavior, saved state formats, and recovery protocols. This replacement changes their execution structure. It does not configure deployment, publication, upstream synchronization, or external credentials. Repository IDs, disabled workflow defaults, placeholder endpoints, pinned images, and migration catalogs remain installation contracts.

## Commands and policy

`operational-runtime.mjs` contains the local subprocess and polling boundaries. A `Commands` instance binds a working directory, environment, and execution function. Script tests substitute a capture function at this boundary; no captured deployment, database reset, release, or restore command is executed. Synchronous command failures stop the current sequence. Polling is reserved for read-only observations and has an explicit deadline.

Readiness evaluates repository identity, branch protection, workflow identity, the newest exact-commit run, and every required job in order. It rereads branch and run identity after collecting jobs. Promotion separately recomputes the candidate and rechecks its immutable reference before the protected fast-forward. Candidate creation uses an isolated bare Git repository, disables hooks and executable Git configuration, and publishes references atomically. Credentials enter only the publication environment. Installed readiness remains a self-contained script, and the local controller pins its bytes.

## Durable release state

The local Python controller separates preparation, build, backup capture, activation, and verification. Every mutation has a persisted attempt phase before it runs. A failed or uncertain mutation leaves that attempt for reconciliation; it is never automatically repeated. Private recovery files retain the prior image, configuration, key hashes, reviewed migration ledger, and backup checksum. Success is published only after the final image, pinned revision, key hashes, access gates, and data audit agree.

The native controller accepts a recent backup attestation and an exact successful build. Its only cloud write selects that build for deployment. The generated workflow still disables autorun and sequences preflight, build, backup, deployment, running-state observation, and verification.

The asynchronous shell controller stores a private operation identity and creation time. A directory claim permits one worker. The installed guard digest, bounded process-group lifetime, exact result framing, and timestamp window are verified before collection. Failed or incomplete output cannot become a success marker. The native guard retains exclusive locking, migration approval, key continuity, access restrictions, read-only audit transactions, backup integrity, and collector readiness checks.

## Development and recovery

The development launcher owns its child set, allocated ports, Compose identity, and API restart state. Existing infrastructure ports are reused only after validating the development Compose metadata. Port allocation remains serial. A restore marker delays API restart until the restore supervisor has finished. Existing local restore keys are reused.

The harness parses arguments and validates its package, immutable images, test target, environment file, and Compose configuration before execution. Archive validation preserves component checksums, sizes, migration ledgers, restore-engine requirements, and artwork contracts. Baseline and candidate image environments are generated separately. State records distinguish baseline verification, upgrade, and completion.

The host restore driver tracks unprepared, prepared, activated, and complete states. Five consecutive database checks precede preparation. A prepared candidate can be discarded; an activated candidate requires rollback and restart of the previous application. Finalization follows health and owner-setup verification. Rollback failure retains recovery state and preserves the original error.

Fresh-install, populated-upgrade, S3, and restore-lab drivers have explicit command and observation boundaries. Their synthetic checks retain first-owner closure, restart persistence, incremental-backfill interruption, session rejection, discarded S3 secrets, restored artwork, immutable package identity, migration-chain refusals, and rollback evidence. Container cleanup targets only each driver's generated project and temporary directory. These drivers remain operational entrypoints: importing them is safe, but invoking their CLI is not a dry run.

## Artwork and provenance

Artwork generation uses fixed composition specifications and separate crop, scale, alpha, and compositing stages. Canvas geometry, matte thresholds, manifest format, public model names, and lossless encoding remain unchanged. Differential characterization compares the prior and replacement implementations on synthetic images and the original source assets, including the manifest and contact sheet.

`scripts/deploy.py` already had replacement provenance before this work and remains unchanged. GPL licensing and historical attribution are retained. This is a behavior-preserving implementation replacement, not a claim of independent or clean-room authorship.

## Validation limits

Original assertions are retained verbatim. Added tests capture operational commands, inject failed transitions, validate archive and path contracts, and compare prior/current restore command traces. Native shell tests run against disposable PostgreSQL with synthetic HTTP responses and credentials. Real cloud deployment, publication, provider login, production restore, and Windows process termination are outside this validation. The repository's disabled release defaults remain in force.
