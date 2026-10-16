# Personalization query and mutation workflows

The dashboard, chart, theme and installation-setting routes retain their existing
HTTP adapters and public contracts. Private modules beneath those route files
plan reads, prepare writes, resolve preferences and coordinate persistence.
The R interface, generated dashboard and chart defaults, theme manifests,
shared validators, schema and private deployment controls remain unchanged.

## Dashboard reads and writes

`dashboards/store.rs` represents visible-list, identity and personal-first slug
reads as explicit query plans. New dashboards and clones prepare an identity,
metadata and config together before issuing their insert. Their conflict policies
remain distinct: create upserts an existing owner/slug; clone reports the existing
validation error for a constraint conflict. A patch prepares config metadata only
when the request supplies a config. Name-only updates retain the prior config.
The regular update's missing-row database error and the administrator update's
404 remain distinct.

`dashboards/reconcile.rs` decodes and writes system defaults in bundled order,
then patches personal managed widgets. Each statement keeps its original commit
boundary. A later system-seed failure leaves earlier successful writes committed
and prevents the personal phase from starting. Revision 9 advances only an older
system default; current, future and personal baseline rows are preserved.

Managed reconciliation removes retired identities first and compacts only the
existing canonical trip layouts. An identity index chooses the earliest widget
matching either bundled ID or managed key. Each match produces a keep, replace or
append operation. Replacement projects managed identity and options while keeping
personal layout, title, visibility and unrelated values. The identity index is
rebuilt after a replacement because a managed-key change can expose a later
match. Appends compute their vertical position from the current widget list.
Only changed configs are persisted.

Create-upsert currently preserves the existing row ID while assigning a newly
generated ID inside an object config. This compatibility behavior is characterized
and retained. No new config validator or metadata repair is introduced.

## Chart selection and mutation

`charts/workflow.rs` prepares creation and clone inputs, patch/config/lock/baseline
changes, and personal-override removal. Each write plan carries the fields needed
by its parameterized statement. Chart creation retains conflict responses, while
patches keep their existing nullable/COALESCE behavior. Reset and administrator
restore resolve the system row before deleting the personal override; they do not
introduce a transaction or retry.

Catalog projection calls the unchanged `chart_registry::merge_entries` first,
then applies enabled and placement filters. A disabled personal override continues
to shadow its system chart. Bundled restoration retains chart baseline revision 5
and does not change lock/default flags. The existing chart validation service,
source manifest and renderer contracts remain authoritative.

## Theme lifecycle and preference resolution

`themes/lifecycle.rs` models save, publish, rollback and retire commands through
three stages: lock the owner-scoped theme and check its ETag; obtain revision
or publication evidence; perform the writes and commit. Retired-state errors
remain action-specific and follow the ETag check. Saving appends an immutable
revision. Publishing preserves the publication's conflict-ignore behavior and
bumps the theme version even when republishing. Rollback selects an existing
published revision without changing the theme version.

Publish-and-apply writes the publication and theme version before checking the
preference ETag inside the same transaction. A stale preference ETag rolls back
all of those writes. Retirement rehomes only matching active custom preferences
to the theme's built-in base. Theme creation continues to lock the user row before
counting the 20 active-theme limit.

`themes/preferences.rs` resolves stored selection evidence before projecting the
existing V2 payload. Missing preferences resolve to Classic dark; an unavailable
custom reference retains the existing fallback reason. Custom selection writes
require an owner-scoped, non-retired published revision. Schema and mode checks
precede transaction admission; ETag checking precedes selection validation.
The shared validators, ETag guards and custom-selection persistence helper stay
in the route module with their existing contracts.

## Installation settings

`settings/workflow.rs` maps a prepared timezone or update-check change to its
persistence operation and audit event. Both writes commit atomically with their
audit record. Stored update-check frequency decoding remains after commit, as in
the original workflow. Administrator and super-user checks remain in the HTTP
adapters. Authentication-provider and security-status adapters are retained.

## Verification and limits

Frozen characterization fixtures cover complete assembled-router responses
(status, every header value and JSON/text body), plus all rows in the affected dashboard,
chart, theme/revision/publication, preference, timezone, backup-setting,
update-check and security-event tables after the sequence steps. The suite covers
162 observations and 75 managed-widget boundary cases, including empty results,
null/scalar configs, foreign ownership, administrator/owner checks, locks,
malformed and oversized requests, chart duplicate races, seed partial progress,
ETag conflicts, publication rollback, retirement and settings audit rollback.

The fixture uses a unique disposable TimescaleDB database and restricted login
role, the current migration coordinator, the production history-protection
function and the production application pool initializer. Bootstrap credentials are confined to fixture database, role and schema
provisioning, extension installation and teardown. Migration and request pools
use the restricted application role. The unchanged
ignored dashboard administrator regression is also executed in its own isolated
database by the scoped harness.

Generated IDs are assigned stable labels, database timestamp fields retain their
presence and nullability but receive a fixed label, and content lengths/rate
counters are normalized in cross-run fixtures. Read and rejected requests also
assert exact unnormalized row equality; successful writes preserve existing creation timestamps. Declared content lengths match actual response bytes. Repeat seeding checks all rows without
normalization. Those tests do not prove real-network/browser behavior or all
possible concurrent schedules. No real accounts, vehicles, provider calls or
production databases participate.

From `apps/api`, with the isolated fixture environment configured:

```sh
cargo test --locked --offline --lib personalization_contract -- --include-ignored --test-threads=1
cargo test --locked --offline --lib routes:: -- --skip personalization_contract
cargo check --locked --offline --all-targets --all-features
```

Ignored or filtered tests are not counted as passes. This replaces the scoped
personalization workflows; it is not a claim that the full application has been
rewritten. GPL-3.0-only licensing and existing NOTICE attribution remain in effect.
