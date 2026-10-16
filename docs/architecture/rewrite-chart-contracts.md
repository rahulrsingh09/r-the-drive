# Chart validation and catalog reconciliation

The backend validates chart definitions and reconciles bundled catalog rows through
private stages beneath `services/chart_validation.rs` and
`services/chart_registry.rs`. Their public definitions, serde rules, errors,
manager records and function signatures retain the existing contracts. The
canonical rendering rules remain in [Chart architecture](../frontend/chart-architecture.md)
and [Chart compatibility and testing](../frontend/chart-compatibility-testing.md).

## Definition validation

Validation runs in this order:

1. Inspect the complete JSON tree for forbidden field names and check the raw
   schema version. An explicit traversal stack preserves depth-first object and
   array order.
2. Deserialize the unchanged `ChartDefinitionV1` types. A shape error ends typed
   validation, after any errors already found in the raw JSON.
3. Check collection sizes, timeframe, unique binding and series IDs, and placements.
4. Validate source capabilities, metric parameters and filters.
5. Validate the global X field, then each series' color, fields, mark, dimensions,
   axis dependency and transforms.
6. Check the total transform count, axes and smoothing.
7. For a known bundled slug, enforce its production renderer contract.

`chart_validation/source_policy.rs` stores each source's field, filter and mark
allowlists in one typed table. A binding index resolves references to the first
occurrence of an ID. Duplicate IDs still produce errors; retaining first-binding
resolution also retains subsequent diagnostics. The metric source continues to
consult the existing series-capable metric registry.

`chart_validation/bundled.rs` compiles the embedded capability manifest and
canonical definitions once with `LazyLock`. Each contract retains its canonical
JSON, series structure without colors, and renderer-specific axis exception.
Validation checks owned metadata, fixed presentation, supported curves and
interaction restrictions in their established order. Raw JSON comparisons preserve
the distinction between an absent field and an explicit null.

The rewrite preserves current acceptance rules, including duplicate schema-version
diagnostics, optional values and existing color-token spelling checks. It does not
introduce a stricter schema or change expression evaluation. Units and field names
are checked as metadata; this layer does not convert telemetry values.

## Seeding and manager resolution

`chart_registry/catalog.rs` converts each embedded seed into a typed `SeedInput`.
The projection retains existing fallback values for missing or mistyped seed
metadata. The generated JSON files remain the source of all 14 bundled defaults,
with baseline revision **5**.

Reconciliation first deletes the retired `charge-session-curve` system row using
both its slug and `owner_id IS NULL`. It then decodes the seed and executes one
parameterized upsert per chart, in seed order. The partial conflict target covers
system rows only. A row advances only when its baseline revision, treating null
as zero, is lower than the incoming revision. An update retains its UUID,
creation time and lock/default flags. Personal rows, current or future baseline
rows, and unrelated system slugs remain untouched.

These statements retain their existing separate commit boundaries. If a later
upsert fails, retirement and earlier successful upserts remain committed. Seeding
does not wrap the catalog in a new transaction or retry writes.

Manager resolution groups rows by slug in a `BTreeMap`, selects the effective
personal override or system row, and derives the existing origin and permission
fields. Output remains sorted by slug. Disabled and locked personal rows still
shadow their system base. The caller must supply rows scoped to the authenticated
user; manager resolution is not an authorization query. Route ownership checks
and history protections are unchanged.

## Characterization and verification

The characterization commit precedes the implementation and preserves the
original tests. Scoped fixtures capture:

- 1,092 valid and invalid definition inputs, including all bundled defaults and
  edits, source/field/mark combinations, bounds, optional and unknown fields,
  forbidden keys, transforms, filters, axes and ordered diagnostics. Accepted
  outputs include the complete typed debug representation; rejected outputs
  include every error path and message in order.
- 128 complete serialized manager results covering input order, ownership,
  disabled rows, locks, system bases and administrator permissions.
- A fresh 14-row catalog, a populated 19-row catalog after reconciliation, exact
  repeat-seed equality, personal overrides, null/old/current/future revisions,
  retired-slug scope, and a later-write failure retaining earlier progress.

Database tests require the dedicated `CHART_CONTRACT_TEST_DATABASE_URL` fixture
endpoint and are explicitly ignored in ordinary runs. They create unique
disposable databases and restricted login roles, apply the authentic migration
coordinator, install production history protection, and connect through the
application pool initializer. Bootstrap privileges install extensions only.
Tests do not disable history protection or grant broad deletion privileges.

Fixtures compare all persisted chart fields. Generated UUIDs and server
timestamps receive stable labels in cross-run snapshots; separate assertions
verify timestamp relationships, preservation of existing IDs and fields, and
exact unnormalized equality after repeated seeding.

Run the focused offline tests from `apps/api`:

```sh
cargo test --offline --locked --lib services::chart_
cargo test --offline --locked --lib services::chart_registry::contract_tests \
  -- --ignored --test-threads=1
cargo check --offline --locked --all-targets --all-features
```

The second command requires the disposable Docker fixture. A skipped database
test is not a successful database check. No vehicle accounts, external providers,
browser sessions or production databases participate in this characterization.
