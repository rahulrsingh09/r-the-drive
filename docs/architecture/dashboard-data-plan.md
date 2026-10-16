# Dashboard data planning and publication

`DashboardDataProvider` coordinates the compact data used by dashboard sensors
and statistics. Its public exports and the `WidgetDef.dataRequirements`,
`WidgetCtx`, and API hook contracts remain unchanged. Charts, maps, tables,
dashboard editing, and private Rust authorization retain their existing owners.

The provider has three stages:

1. `dashboardQueryPlan.ts` combines widget demands and captures query arguments.
   Metric names are sorted and deduplicated; latest values default on and series
   default off when merging widget declarations. A demand from any widget wins
   over another widget's disabled flag. Direct provider callers retain their
   original optional metric flags because the batch hook owns those defaults.
   Query plans copy and freeze metric requests, summary vehicle targets, and
   trip-tag filters without freezing the caller's objects. The existing hooks
   still own cohort normalization, cache keys, authentication, and network access.
2. `dashboardData.tsx` admits source results and projects metric values and
   series into lookup maps. Disabled summary sources and an empty metric manifest
   publish no cached data. Missing responses clear prior projections. Zero,
   null, series-only, and value-only readings retain their original meanings.
   Maps are built once per admitted batch result, so a live-status change does
   not rebuild metric selections. Maps and query-owned result objects are
   treated as read-only; the provider never mutates them after projection.
3. Each selected vehicle has a store initialized with its current projection.
   The provider publishes subsequent snapshots in a layout effect, after React
   commits and before the browser paints. A suspended or abandoned render cannot
   change the store observed by the committed tree. Changing vehicles replaces
   the store without remounting widgets. Selector results are cached within a
   snapshot and subscribers update only when their selection changes.

## Vehicle and placeholder boundaries

The shared query hooks may return the previous key's data as a placeholder.
Each source records its last confirmed vehicle only after a non-placeholder
result commits. A placeholder is admitted only for that same vehicle; changing
the date range or cohort can therefore retain useful data while refreshing.
Switching vehicles withholds old placeholders independently for each source
until that source supplies data for the selected vehicle. Status payloads with
an explicit different `vehicle_id` are rejected; partial status snapshots retain
the hook's query identity. Clearing the vehicle withholds all data.

These checks complement hook-level query keys; they do not authorize a user or
replace backend membership checks. Metric and summary payloads lack a vehicle ID,
so non-placeholder result identity continues to rely on the existing query hooks.
The refresh flag retains its existing OR across the five hook fetch states.

## Verification

The data requirement and provider contract tests preserve the established
request and display behavior. Additional cases cover immutable plans, disabled
cached results, rapid vehicle switches, same-vehicle placeholders, status
identity, metric subscription stability, allocating selectors, and suspended
render isolation.

```sh
pnpm -C apps/web exec vitest run src/test/dashboardDataRequirements.test.ts src/test/dashboardDataProvider.contract.test.tsx src/test/dashboardDataTagFilters.test.tsx src/test/dashboardQueryPlan.test.ts
pnpm build
pnpm docs:check
pnpm docs:build
```
