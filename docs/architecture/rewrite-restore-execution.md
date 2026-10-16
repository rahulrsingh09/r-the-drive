# Restore execution and durable replay

This work replaces activation coordination in `panda-restore-agent` and the
operational-history replay scheduler. The command names, HTTP handlers, job
serialization, host-state serialization, phase messages, progress values, and
public service signatures remain compatible with the characterized baseline.
Documentation impact: internal architecture update.

## Activation and rollback

The runtime executor advances through stop, database swap, settings notification,
artwork activation, restart, and health verification. It tracks which resources
have actually changed: the candidate is still inactive, the database has switched,
or both the database and artwork have switched. A failed stage selects its
compensation from that state. Failures before a successful swap return directly;
later failures enter the existing rollback protocol with precisely the artwork
handle that was activated.

Host activation uses the same resource states with its own persisted intent and
completion checkpoints. An already activated host request remains a no-op.
An interrupted activation still requires `host-rollback`. The host-state format
and its existing persistence behavior are unchanged.

Runtime rollback executes an ordered sequence: record rollback intent, stop the
application, restore the previous database, restore any activated artwork, release
the restart marker, verify health, and record successful rollback. Existing error
precedence, best-effort operations, retry eligibility, and terminal messages are
preserved.

The settings phase still reports progress 70 after the database-swap phase reports
74. This is part of the existing observable contract; this change does not silently
make the values monotonic.

## Operational-history replay

One blocking parser owns the three dependency-ordered passes: runs, artifacts,
then restore requests. Each parsed record carries a completion receipt. The async
consumer invokes the unchanged catalog transaction and acknowledges only after
that transaction succeeds. The producer cannot parse another catalog record or
start the next pass before receiving the receipt.

Each pass still opens the input, checks its length, applies the bounded reader,
rejects duplicate selected fields, and rejects trailing JSON. A database error
closes the receipt channel and stops parsing. Cancellation drops the receiver and
any outstanding receipt. Records committed before a later parsing or relational
error remain committed, as in the baseline. Source artifact availability remains
inert, and verified target history retains its final merge precedence.

## Retained boundaries

Archive admission, extraction, candidate schema validation, journal implementation,
catalog merging, recovery workspaces, key custody, and private history policies are
unchanged. Candidate preparation still executes the archive under its restricted
temporary role and removes login capability and the password afterward. The
recovery mutation lock, deadline cleanup, pinned package checks, safety backup,
schema/transform preflight checks, and process-command adapters remain in place.

Migration 0031, charge row locking and pin checks, durable detector representations,
and ingestion worker code are unchanged. The new tests call the existing worker
entrypoint to validate completion delivery; they do not replace or bypass its
collector locks or persistence code. GPL-3.0-only licensing, NOTICE attribution,
private deployment behavior, and the R interface are retained. This work is informed
by the existing implementation and is not an exclusive-authorship or clean-room
claim.

## Characterization and validation

The baseline characterization was committed before the runtime replacement. Test
code and frozen JSON live in `apps/api/src/bin/__tests__`. Evidence output is
optional; frozen assertions always execute. Disposable database tests remain
explicitly ignored unless their isolated database configuration is supplied.

Coverage includes:

- Complete job HTTP bodies and ordered authentication/path errors, oversized
  capability headers, artwork activation/rollback, and bounded settings parsing.
- Complete operational-history rows across repeated replay, empty and unknown
  fields, null/malformed/oversized input, duplicates, trailing data, and late
  foreign-key failure.
- Actual PostgreSQL candidate preparation, unprivileged object ownership,
  database swaps, rollback, recovery-lock contention/release, and the existing
  archive-execution timeout regression.
- All existing injected runtime fault points, successful runtime restore, host
  prepare/activate/rollback/finalize, repeated host activation, and retry admission.
- Nonempty trip checkpoints, trip completion outboxes, charge checkpoints, and
  charge completion outboxes for two separate users and vehicles. Full rows and
  canonical references are compared across rollback and successful restore.
  Active and pending charge pins reject deletion.
- Real worker startup without provider credentials, durable completion delivery,
  stale-trip finalization at its captured observation, and a second startup with
  identical persisted results after explicit clock normalization. Active charge
  continuation compares complete serialized completions, including encoded NaN,
  signed zero, and precise finite float bits.

The fixture installs authentic migrations, bootstraps extensions separately, and
uses restricted application permissions with history protection. PostgreSQL
administrative operations apply only to fresh disposable fixture databases.
Health responses come from a loopback server; no provider connection is needed.

Full raw journals, catalogs, process logs, and state snapshots are retained outside
the repository. Frozen complete HTTP, catalog, candidate-row, retry-journal, and
durable-state assertions are supplemented by runtime/host phase and relationship
assertions. Random backup bytes and their derived checksums are retained as raw
evidence, not normalized into a claim of byte-identical full backups.

## Limits and integration

Existing early failures after candidate validation, safety backup, or history
merge can leave an inactive candidate for later cleanup. Tests record that
behavior and remove only their disposable candidates. Database deletion can race
Timescale background sessions; fixture cleanup retries do not change production
cleanup behavior. Host-state file writes, hard process termination at arbitrary
instructions, and deadline expiry after cutover retain their prior semantics;
the fault matrix is not an exhaustive crash-consistency proof.

The documentation owner must register
`architecture/rewrite-restore-execution` in the shared sidebar. This bounded
change does not authorize a production restore, release, deployment, or a claim
that the entire application has been rewritten.
