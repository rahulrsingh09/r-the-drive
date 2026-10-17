# Dashboard Authoring Guide

## Purpose

Use this maintainer guide when adding a bundled dashboard or widget. The application displays fixed layouts and existing saved dashboards; it does not expose layout editing.

The main goal is to keep future work inside the shared dashboard seams instead of reintroducing route-level duplication.

## Before You Add Code

Decide which layer owns the change.

| Change                                              | Put it here                         |
| --------------------------------------------------- | ----------------------------------- |
| New route path, URL params, route-local composition | `apps/web/src/routes`               |
| Shared page scaffolding, view state, common actions  | `apps/web/src/components/dashboard` |
| New dashboard widget or default config              | `packages/dashboards/src`           |
| New API query hook or response normalization        | `packages/hooks/src`                |
| Reusable chart, table, badge, primitive             | `packages/ui/src`                   |

If the change crosses more than one row, start from the lowest reusable layer and move upward.

## Adding a Built-In Dashboard Page

1. Decide whether the page is mostly a dashboard grid or a custom detail page.
2. If it is a dashboard page, use the shared `DashboardPageShell` path instead of creating a new page scaffold.
3. Keep the route wrapper thin. It should only:
   - declare the path
   - read params or search state
   - pass explicit page composition pieces
4. If the page needs a hero panel, tabs, or a summary strip above the grid, render that through a page composition hook such as `renderBeforeDashboard` or a page-local wrapper component.
5. Update the default dashboard config under `packages/dashboards/src/defaults/`, then run `pnpm dashboards:sync-defaults` to generate the backend seed copies. Verify with `pnpm dashboards:sync-defaults --check`.

## Adding a Widget

1. Add or reuse a hook in `packages/hooks/src`.
2. Add or reuse a reusable chart/table/card in `packages/ui/src` if the visual is generic.
3. Create the widget module in `packages/dashboards/src/widgets/`.
4. Register it via the existing widget barrel and registry.
5. Keep the widget focused on one concern.
6. Preserve the widget definition and geometry contracts needed by existing saved layouts.
7. Declare `dataRequirements(instance)` in the registry entry for every dashboard-wide source or metric chip. Return only the sources the widget actually reads; metric entries should state whether they need a latest value, sparkline series, or both.

A widget should usually:

- read a dashboard data selector or call one hook or a small related set of hooks when the data is intentionally widget-specific
- map data into one visual or table
- avoid knowing about route params beyond the provided widget context

Do not use one widget to coordinate the rest of a page.

### Sensor Background Graphs

Sensor widgets can render a compact background graph through `options.chartType`.
Use `line` or `area` for curve interpolation and store the amount in
`options.curveSmoothing` as a number from `0` to `1`:

- `0` renders straight segments.
- A positive value renders a cubic curve when at least three finite data points are available.
- One- or two-point series render straight because there are not enough neighboring points to define a curve.
- `bar` and `daily_delta` renderers do not use curve smoothing.

This setting changes the visual path between samples; it is not a moving-average
or other data-smoothing operation. Keep the option on the shared sensor widget
contract so bundled configurations and existing saved dashboards render consistently.

### Dashboard Data Requirements

`WidgetDef.dataRequirements` is the declarative request contract for shared dashboard data. `DashboardRenderer` collects it from the visible configuration, deduplicates metrics, and creates one dashboard provider. This applies equally to built-in and user-created dashboards.

```ts
dataRequirements: (instance) => ({
  status: true,
  metrics: [{ metric: 'odometer_mi', include_latest: true, include_series: false }],
})
```

Use `useDashboardDataSelector` or `useDashboardMetric` in the widget to read that data. Keep specialized sources—trip geometry, trip detail samples, table paging/search, and active chart sources—outside the generic metric batch. Unknown or older widget definitions safely contribute no requirements and retain their own data behavior.

### Saved widget geometry

Preserve geometry validation and legacy registry metadata required to read saved
configurations. These fields do not enable a widget palette, movement, resizing,
or editing controls. New bundled geometry belongs in the default configuration
and must work on desktop and mobile without user rearrangement.

## Adding A Table

Prefer the shared table seam before inventing page-local table chrome.

Use the existing shared pieces in this order:

1. `packages/ui/src/tables/DataTable.tsx` for the actual table body, sorting, empty state, and optional column visibility menu.
2. `packages/ui/src/tables/TableControls.tsx` for the standard search, rows-per-page, and pagination chrome.
3. `packages/ui/src/tables/*Columns.tsx` for column definitions that keep widths compact and readable.

Table rules:

- Keep page-local wrappers `min-w-0` and avoid horizontal scrolling as the default fix.
- Use fixed or explicit widths for dense tables before adding more columns.
- Collapse related values into one cell when the table is getting too wide, like `SoC start -> end`.
- Default-hide secondary columns instead of widening the table for rarely used data.
- Use tooltip headers for domain-specific labels so the cell text can stay compact.
- If a new table is not a good fit for `DataTable` plus `TableControls`, document why in the code and reuse the nearest shared seam rather than creating a one-off chrome block.
- If a table shows inferred vehicle behavior, define the canonical metric, exclusion rules, and evidence thresholds before wiring the UI. Do not render low-confidence derived values as if they are first-class facts.
- Prefer nullable secondary metrics over forced placeholders when the underlying evidence is weak. Phantom Drain is the model here: SoC-backed drain is canonical, range loss is derived, and state-based sleep share is hidden when coverage is weak.
- When behavior is inferred from vehicle lifecycle events, keep the source-of-truth hierarchy explicit: completed trips and charge sessions are canonical facts, while state periods and raw telemetry are overlays that annotate or validate those facts. Do not let noisy state data become a first-class session builder unless the feature is explicitly about state history itself.

When adding a new table page, make the search box and pagination semantics match the existing site patterns first. Only diverge when the data shape makes the shared controls genuinely misleading.

## Sensor Chip Language

Use `componentType: "sensor"` for compact stat chips unless the card owns a genuinely custom interaction or visual. The reusable sensor chip supports:

- Metric catalog values through `metric`, `valueMode`, and optional background graphs.
- Direct page data through `dataSource`: `batteryHealth`, `chargingSummary`, or `vehicleStatus`.
- Simple math through formulas such as `([home_kwh] / [total_energy_kwh]) * 100`.
- Composite display pieces through paths, formulas, and templates such as `/[usable_new_kwh:kWh]` or `Home [home_kwh:kWh] / Away [away_kwh:kWh]`.
- Theme-owned data-category color through `dataAccent` and `valueColor: "data"`.
  The active theme resolves the automatic series slot; explicit `curveColor`
  and style choices remain user overrides. Charging and Battery definitions
  should use the shared category mapping instead of route-local colors.

For `vehicleStatus` chips, the definition also owns the semantic availability behavior.

- `current` fields render the live semantic value.
- `historical` fields render the last known semantic value plus a small `Last updated ...` line.
- `never_seen` fields render the shared blue `Unavailable` chip with tooltip context when available.
- Composite status chips such as windows or tonneau should treat any current subfield as current, otherwise fall back to historical if any constituent field has been seen before.

For source-backed chips, define the canonical behavior in `packages/dashboards/src/widgets/sensor/sensorDefinitions.ts`. Keep default dashboard JSON minimal when a definition already owns the source, unit, formula, inline secondary, label suffix, icon, value color, and graph defaults. Use widget `options` only for presentation overrides such as an accent border. Conditional visibility belongs in the typed `visibility` field, not widget options.

For `timeframeScope: "range"`, the chip value is the server-calculated aggregate for the dashboard's selected dates; its sparkline is context only and must not replace that aggregate. Derived measures should aggregate their components directly (for example, distance-weighted consumption), and expose coverage or a tooltip when incomplete source data affects confidence.

Maintainers declare the sensor settings in the widget configuration:

- `Data source` chooses the backing object or metric catalog.
- `Value path`, `Fallback path`, `Formula`, and `Unit` define the primary value.
- `Label suffix`, inline fields, and `Secondary line` define compact composites while preserving the existing chip layout.

Example usable-capacity definition:

```json
{
  "dataSource": "batteryHealth",
  "valuePath": "usable_now_kwh",
  "unit": "kWh",
  "inlineSecondaryTemplate": "/[usable_new_kwh:kWh]",
  "labelSuffix": "now/new",
  "valueColor": "default"
}
```

## Adding Page-Level Composition

Page-level composition is for content that should not live inside the dashboard grid.

Use it for:

- Overview hero content
- page-level tabs
- CTA banners
- detail summaries

Do not implement this by branching on dashboard slug inside the shell or renderer. Pass it in explicitly from the route or route-local wrapper.

## Fixed-layout rules

`DashboardPageShell` fetches the selected saved configuration and coordinates the
page timeframe and ordinary viewing actions. It does not create edit drafts,
render save/discard controls, or accept an edit query parameter as permission to
change the layout. Keep the same boundary for administrators and ordinary users.

Settings > Dashboards supports opening and exporting existing rows. Do not add
create, customize, import, duplicate, reset, delete, restore, lock, or layout-edit
controls. Chart definition editing and account-backed chart favorites have their
own supported workflows and remain available.

### Conditional visibility

Widgets may carry typed visibility rules, for example:

```json
{
  "visibility": [
    { "type": "vehicle-connection", "value": "plugged" }
  ]
}
```

Multiple rules use AND semantics. An absent or empty rule list means always
visible. Resolve these rules from live status through
`packages/dashboards/src/dashboardVisibility.ts`; connected standby counts as
plugged in. Keep hidden widgets in the saved configuration. There is no editable
scenario preview or layout draft in the interface.

## Bundled baseline and saved-layout contract

The five system dashboards are authored in
`packages/dashboards/src/defaults/`. Files under `apps/api/dashboards/` are
generated seed copies; run `pnpm dashboards:sync-defaults` and verify with
`pnpm dashboards:sync-defaults --check` after changing a bundled default.

Startup creates missing system dashboards and applies an explicitly newer
bundled revision once. Existing personal dashboards retain their widgets,
positions, titles, visibility, and settings. Narrow, versioned compatibility
updates for widgets marked `managed` with a stable `managedKey` remain server
responsibilities; do not inject replacements into a saved layout during render.

The normal route selects a same-slug personal dashboard before the system row.
An exact `/d/$slug?dashboardId=<uuid>` lookup must fail inline if unavailable,
rather than silently showing another row. Preserve configuration sanitization
and saved geometry without mounting a drag/resize grid or mutation controls.

## Data and Adapter Rules

If several widgets on one page need shared derived data, create a page-specific hook or adapter instead of duplicating fetch logic in the route and widgets.

Good examples:

- an overview hook that returns live state, last drive, and last charge together
- a costs summary hook used by several cost widgets

Bad examples:

- multiple widgets each reimplementing the same response normalization
- route wrappers doing heavy transformation that should live in hooks

When a chart is presented on both a dashboard and a dedicated detail page, share the typed chart component and data transform. Both surfaces should consume the same validated source rows and timeframe rather than maintaining separate chart-only aggregation paths.

Chart authoring is curve-first for rich time-series and independent charts. The standard editor exposes **Basics**, **Curves**, **Display**, and **Advanced**; it derives a compatible shared domain from selected data groups while incompatible curves remain visible with an explanation. Fixed-geometry bundled renderers expose only controls their production component consumes. Their X encoding, curve structure, mark, axes, display, and interaction contract are validated as immutable; use a custom slug when different geometry is required. A chart may combine up to four compatible data groups. Imported mixed-domain custom definitions remain unchanged and are edited through Advanced rather than silently normalized.

Preview the unsaved definition through `ManagedChartRuntime`, the same production registry and runtime adapter used by assignment-driven dashboard catalogs. A bundled slug always retains its established specialized renderer. Rich renderers receive supported series/display edits; fixed-geometry renderers receive only their declared capability subset, and frontend/API validation rejects everything else. Placement metadata only controls where the chart is eligible. Database definitions are authoritative; portable import and export serialize that same validated structure. Keep explicit regressions for renderer-family routing, required X encoding, preview/dashboard parity, and rejected no-op edits.

## File Placement Checklist

### New built-in dashboard page

- route file in `apps/web/src/routes/`
- optional page-local composition component in `apps/web/src/components/dashboard/` or a page-specific route folder
- default config in `packages/dashboards/src/defaults/`
- new widgets in `packages/dashboards/src/widgets/`
- new hooks in `packages/hooks/src/`

### Existing saved-dashboard support

- shared shell behavior in `apps/web/src/components/dashboard/`
- dashboard framework support in `packages/dashboards/src/`
- avoid putting framework behavior directly in `apps/web/src/routes/d.$slug.tsx`

## Review Checklist

Before merging dashboard frontend work, verify:

1. The route stayed thin.
2. Shared shell behavior was reused where possible.
3. No new slug-based branching was added to the shell or renderer.
4. Generic UI stayed in `packages/ui`.
5. Data access stayed in `packages/hooks`.
6. Built-in and saved personal dashboards render without layout-editing controls or edit-link bypasses.
7. Empty, loading, and error states still exist.

## When To Introduce a New Abstraction

Introduce a new shared abstraction only when at least two dashboards or routes need it.

Preferred order:

1. reusable UI primitive
2. reusable hook
3. shell-level composition prop
4. framework-level abstraction

Do not jump straight to a new framework primitive if a page-local component or hook is enough.
