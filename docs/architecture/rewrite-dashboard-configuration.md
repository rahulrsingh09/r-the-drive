# Dashboard configuration transformations

The dashboard configuration seam preserves the existing saved formats and
public functions in `packages/dashboards/src/layout.ts`, `yaml.ts`,
`charts/expressions.ts`, and `charts/portable.ts`. The transformation stages
separate validation, compatibility normalization, and browser downloads.
The implementation builds on the existing GPL-3.0-only project; its attribution
and NOTICE remain unchanged.

## Layout

`sanitizeDashboardConfig` validates the whole saved document before applying
widget registry constraints. Invalid persisted coordinates, fractional
dimensions, or unsupported schema versions are rejected rather than repaired.
Unknown schema fields are stripped; arbitrary widget `options` remain intact.

`sanitizeWidgetInstance` resolves registry capabilities. Fixed widgets use
their default width and height; resizable widgets use their declared bounds.
Unknown definitions retain their identity and use the fallback minimum of
one cell per dimension.

`sanitizeWidgetLayout` derives dimension bounds, projects width and height,
then projects horizontal position into the remaining twelve-column space.
Vertical position is rounded and clamped to zero without a maximum.
Direct callers can supply unsanitized geometry: nonfinite dimensions use the
lower bound, and nonfinite positions use zero. Bounds are applied after
rounding, so fractional or contradictory bounds retain the historical
fractional-result and upper-bound precedence behavior. Fixed width is still
capped at twelve columns. No stage mutates its input.

## Dashboard YAML

Export selects only the version, slug, name, description, controls, and widgets,
then uses the existing YAML serializer and `# Panda dashboard v2` header.
It does not validate or normalize an in-memory configuration before export.
The serializer owns escaping, quoting, Unicode, and multiline values.

Import decodes YAML, overrides identity and ownership to create an unlocked,
non-default copy, and validates and sanitizes that candidate. Only then does
it invoke `normalizeDashboardConfig` from the existing API module for legacy
trip widgets, connection visibility, and charging-height compatibility.
The initial validation prevents normalizer defaults from accepting missing
controls or widgets. JSON documents remain accepted as YAML input.

Schema parsing can reorder widget fields on the first import. Values round-trip,
and re-exporting a normalized document is stable. Exported widget IDs remain
unchanged; each import receives a new dashboard ID. The download wrapper
retains the slug-based `.yaml` filename and `text/yaml` MIME type.

## Arithmetic

The expression pipeline binds bracketed paths as text, tokenizes decimal and
scientific numeric literals, translates operators to postfix instructions, and
evaluates those instructions with a numeric stack. Unary signs bind more
tightly than multiplication and division; binary operators associate left.
No JavaScript evaluator or executable expression syntax is used.

Both the original formula and the substituted text have a 512-character
limit. Empty, malformed, missing-value, zero-divisor, and nonfinite-result
expressions return `null`. Finite literals may overflow during arithmetic;
the final result must be finite. For example, dividing one by an overflowing
positive product still yields zero, preserving existing behavior.

Binding intentionally retains textual semantics: adjacent references can
concatenate digits, a substituted negative value introduces a minus sign,
and substituted negative zero becomes `"0"`. Paths traverse numeric members
of objects and arrays, including inherited members and empty segments.
Bracket references trim the complete path; direct path lookup does not.
These are compatibility rules, not new authoring recommendations.

## Portable charts

Export projects a record into the `panda-chart` version-one document and
serializes JSON with two-space indentation. Ownership, IDs, lock flags, and
baseline revision do not travel with the chart. Export does not validate.

Parse checks UTF-8 byte length before JSON decoding and delegates validation
to `PortableChartDocumentSchema`. Exactly 256 KiB is accepted. Unknown
structured keys are stripped, source `params` retain their arbitrary values,
and the definition schema rejects forbidden keys in retained values.
This boundary does not validate live source availability, bundled renderer
capabilities, or expression syntax. Those remain separate caller concerns.

Import maps portable fields to a non-default, unlocked draft and validates
with a schema derived by omitting record identity fields from
`ChartRecordSchema`. It needs no fabricated UUID. Explicit typed projections
omit absent descriptions because shared Zod schemas infer optional
`undefined` values while the transport interfaces use exact optional fields.
Empty descriptions are omitted on export; whitespace descriptions are
trimmed by parse and omitted by import when they become empty.

Slugs, renderer IDs, placements, and visual definitions survive transfer.
This seam does not select or modify renderers, migrate bundled chart
baselines, or rewrite personal rows. The download wrapper retains
`.panda-chart.json` and `application/json`.

## Verification and dependencies

`dashboardConfigurationLayout.test.ts`,
`dashboardConfigurationExpressions.test.ts`, and
`dashboardConfigurationTransfer.test.ts` use synthetic fixtures to characterize
the original implementation and verify the replacement. They cover malformed
input, round trips, escaping, registry bounds, immutability, metadata removal,
schema defaults, formula limits, and the exact UTF-8 transfer limit.
Existing chart contract, renderer, editor, dashboard API, registry, visibility,
and grid tests exercise the surrounding seams.

The shared dashboard/chart schemas, widget registry, API normalizer,
visibility migration, consumers, and rendering components remain dependencies
owned outside this transformation layer. In particular, moving the pure
normalizer out of the API module would require a coordinated change; YAML
import continues to use it here. No public signature or persisted schema
changes are required.
