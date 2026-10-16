---
title: Charge payload reconciliation
---

This page describes the charge payload recording and summary enrichment workflow.
The public Rust entrypoints, serialized summary fields, provider requests, SQL schema,
and charge durability contracts remain unchanged. Documentation impact:
**Documentation site update required**.

## Payload resolution

`ingestion/rivian_poll/payload_recording.rs` resolves one incoming payload into a
plan before applying writes in its transaction:

1. Compute the existing database identity key and semantic fingerprint, then lock
   any identity row.
2. Validate identity metadata and its canonical payload. A matching canonical
   payload wins; a missing stored fingerprint can be filled without changing the
   returned repair flag.
3. If necessary, select an exact surviving payload by vehicle, operation, nullable
   provider identifiers, and semantic fingerprint. A valid same-vehicle session
   link ranks first, followed by capture time and payload UUID.
4. Reuse, repair, expire, claim historical evidence, or insert new evidence. Repair
   updates the identity and its alias references in the same transaction.
5. Commit once after the selected operation completes.

An expired durable identity remains a tombstone. Replays do not manufacture a new
JSON payload when every matching retained payload is gone. A historical payload
without an identity can be claimed without inserting another JSON copy. Concurrent
new inserts keep the winning identity and remove the losing speculative payload
inside the transaction. Historical-claim and new-insert conflict paths retain their
existing distinct behavior, including their repair flags.

Database identity functions, exact-survivor filtering, alias repair predicates and
retention reference cleanup remain existing adapters. The bounded identity backfill
worker and retention reconciler are unchanged. This work does not replace those
previously completed stages.

## Summary matching and persistence

Matching still attempts external aliases, grouping keys, overlap and nearby time
windows in that order. Alias and grouping queries now use existence predicates
instead of joining and deduplicating candidates. They deliberately retain lowest
session UUID selection; their historical ordering does not globally prioritize
source quality. The overlap query retains its source, distance, start and UUID
ordering.

Nearby matching selects the first two candidates under the existing source and time
ordering, then checks the distance gap. Both window endpoints are inclusive. A gap
of exactly 900 seconds admits the first candidate; a smaller gap rejects it. Source
priority can place a more distant candidate first, so the signed gap remains
significant.

Summary persistence locks the selected session and projects its proposed fields
once. A complete row comparison suppresses an update when none of those fields
change. Existing non-null values continue to win, including zero, false and empty
strings. API bounds widen; telemetry bounds stay intact. API-only completion and
duration use the row's existing source. Manual location selection prevents inferred
home status from replacing it. Duration retains PostgreSQL's seconds-to-integer
rounding followed by integer division, including fractional and negative intervals.
The source and confidence parameters retain the caller's existing-source contract.

Session enrichment and alias persistence remain separate operations. A late alias
failure may leave the session enrichment committed; retry completes the aliases.
Payload recording, by contrast, rolls back its payload, identity and alias writes
together on a late failure. No broader request transaction is introduced.

## Retained boundaries

The charge history lock, row revalidation, active/pending pins, checkpoint and
completion tables from migration 0031, and `charge_sessions/durability.rs` are
unchanged. Canonicalization still defers pinned clusters. Completed graph reduction,
replay, repair and backfill stages retain their interfaces and behavior.

Provider cancellation, request catalog, response limits, authentication, credential
encryption, outbound policy and private deployment/history policy remain unchanged.
These internal persistence functions do not add an HTTP authorization surface or a
new payload-size policy. The existing guarded provider and route callers continue
to own those checks. UI, schema, migrations, dependency locks, GPL-3.0-only licensing
and NOTICE are unchanged.

## Verification

Frozen fixtures under `apps/api/src/services/__tests__/payload_reconciliation_goldens`
were captured from the original implementation and verified against it before
replacement. They compare all stored session, payload, identity, alias, telemetry,
checkpoint and pending-completion fields, plus ordered outputs and errors.
Generated payload UUIDs and runtime clocks are explicitly normalized; fixed
historical timestamps remain exact. External review evidence retains raw values.

Synthetic database tests cover survivor and alias collisions, null and empty values,
fractional units/timestamps, exact ambiguity boundaries, multi-vehicle isolation,
corruption rejection, bounded retention, duplicate replay, late write failures,
retry, concurrent insertion, cancellation while waiting for an identity lock, and
durable pin deferral. Existing poll, charge-history, identity-index and library
regressions remain present.

The database fixture requires an explicit `PAYLOAD_DATABASE_URL` for a newly created
local installation. Privileged bootstrap installs extensions separately; migrations
and workflow operations use a non-superuser application role with production history
protection enabled. No history permission is relaxed. Provider tests use existing
local fake transport seams.

Cancellation testing drops an in-flight future and retries through the database;
it is not a process-kill or network-partition experiment. This bounded rewrite does
not establish complete application or live-provider parity.
