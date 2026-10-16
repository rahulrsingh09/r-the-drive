# Historical charge reconciliation

The historical charge services preserve the R interface and the existing
PANDA/panda data contracts. Provider identifiers, persisted payload identities,
session IDs, annotations, and repair journals keep their existing meanings.
The GPL-3.0-only license and existing NOTICE attribution continue to apply.

## Reconciliation stages

`charge_sessions.rs` builds normalized summary evidence once: sorted grouping
keys, ranked external aliases, charger type, and network classification. It
then searches in this order:

1. An external alias or canonical provider ID on the requested vehicle.
2. A normalized grouping key on that vehicle.
3. An overlapping session, using the existing source and time ranking.
4. A start within sixty minutes, provided the next candidate is at least
   fifteen minutes less suitable under the existing ranking.

Existing alias and grouping queries retain their UUID-first ordering.
Normalization preserves case, provider ID spelling, and the historical
adjacent-only alias deduplication rule. A missing start prevents an unmatched
insert but does not prevent an alias match.

Summary enrichment fills missing saved values and widens API evidence times.
It preserves telemetry boundaries, saved values, and manual location choices.
Energy in kWh, energy in Wh, and range in km remain separate fields. Unknown
values remain nullable; vendor absence differs from a known non-Rivian vendor.
The API-only insert and enrichment paths retain their existing duration
rounding rules.

Canonicalization discovers connected components through shared external IDs,
grouping keys, telemetry restart transitions, and effective time overlap.
An iterative traversal discovers time edges without retaining a quadratic
adjacency matrix. Members retain their input order so survivor selection and
duplicate field precedence remain stable. A single reduction collects local
and API boundaries and the preferred alias before persistence.

The existing persistence statements remain responsible for field-level
precedence, user annotations, reference reassignment, and duplicate deletion.
Broad historical canonicalization retains its existing nontransactional
boundary. It does not acquire the startup repair journal's transaction
guarantees.

## Payload identity and backfill

`charge_payload_identity.rs` claims bounded pending evidence using
`FOR UPDATE SKIP LOCKED`, fills fingerprints, inserts durable ledger entries
from the typed returned rows, and records progress in the same transaction.
The database functions `panda.charge_payload_fingerprint` and
`panda.charge_payload_identity_key` remain the only identity definitions.
Fingerprinting therefore preserves PostgreSQL's JSONB numeric representation,
including its distinction between `1` and `1.0`.

A short or empty claim requires an exact pending count before completion.
Advisory exclusion, retry delays, batch limits, index selection for hypertables,
and retention reconciliation remain unchanged. Retention clears missing
evidence pointers independently for identities and aliases; it keeps durable
identities and does not manufacture replacement JSON.

`charge_backfill.rs` separates the atomic vehicle claim from fetch completion.
The completion pipeline awaits the existing provider fetch, canonicalizes the
requested vehicle, counts saved sessions, and records the outcome. A failed
fetch or reconciliation records an error on a best-effort basis without
replacing the original error. Failure to persist a successful result remains a
database error. Provider access itself remains in the existing ingestion code.

## Startup repair

`charge_session_repair.rs` keeps a single transaction around restart repair,
reference reassignment, journal images, and terminal closure. It retains the
existing restart eligibility query and journal revision. Terminal closures are
applied as one parameterized batch with the original repair keys.

Unassociated Parallax sample reconciliation reads a bounded assignment plan
and records attachment and its checked marker together where possible. Only
one matching closed session permits attachment; ambiguous and unmatched
samples are marked for later reconsideration. Cursor ordering, one-hour retry
eligibility, late historical samples, and segment distinctions are preserved.

Rollback reads a typed journal entry, verifies the saved after-image and
reference mappings, restores the recorded rows, and marks the journal reverted
within one transaction. Changed references or saved rows still refuse rollback.
Terminal-close journals retain their historical format; the merge rollback
interface does not gain support for undoing them.

## Verification

The owned modules contain synthetic database contract tests in addition to
their existing tests. Run them only against a disposable TimescaleDB instance
whose credentials may create and drop test databases:

```sh
SQLX_OFFLINE=true cargo test --offline --locked --lib services::charge_ -- \
  --include-ignored --test-threads=1
cargo check --offline --locked --all-targets
```

The contract fixtures assert complete saved session rows, relationships,
source precedence, units, invalid input handling, isolation between vehicles
and members, replay behavior, and transactional failure and rollback. Optional
test-only `CHARGE_HISTORY_EVIDENCE_DIR` and `CHARGE_HISTORY_ORACLE_DIR` paths
capture and compare complete database outputs. Only processing-clock fields
are replaced by a marker; session times, provider IDs, numerical values,
nulls, fingerprints, and reference keys remain exact.

The backfill completion tests can supply a synthetic fetch future. They do
not call a live provider. These tests do not establish production-scale
performance or parity for every concurrent ingestion interleaving.
