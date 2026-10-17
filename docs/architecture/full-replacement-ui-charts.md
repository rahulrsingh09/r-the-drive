# Chart implementation boundaries

The specialized chart components remain the rendering contract for bundled charts.
Their Recharts trees, SVG attributes, canvas operations, labels, units, and responsive
classes are preserved. This implementation does not change bundled charts to the
generic definition renderer.

Trip charts index measured columns together through `tripMeasurements.ts`. Their
marker selection and hover resolution use the same measured observations; tooltip
field aliases and formatting are compiled by `chartFormatters.ts`. Tire history joins
sample timestamps with trip boundaries before constructing pressure columns.

`dailyChargingModel.ts` groups chronological sessions by day and charger type, then
joins daily totals and computes stack geometry once. The chart maintains independent
hover, selection, and zoom transitions. Empty days, missing costs, session order, and
day-click suppression after range selection retain their previous behavior.

`RichTimeSeriesChart` remains the canonical uPlot renderer. It builds a stable sample
permutation, indexes stacked series once, and computes stack bases before emitting
paths. `richCursor.ts` separates controlled cursor writes from notifications and
coalesces resize work. Native stepped and spline paths still come from uPlot; both
curved settings intentionally use the same native spline. Per-series gap overrides
and the left-axis presence rule from the prior implementation are preserved.

`timeFilter.ts` partitions finite samples into monotonic timestamp runs. Its moving
window deliberately preserves the existing recurrence: departures subtract the
already-emitted reading. Changing this rule requires a separate behavioral change.
`sparklineGeometry.ts` prepares geometry and curve commands once, then replays those
commands for fill and stroke. Continuous-gap limits, singleton dots, and bar buckets
remain specific to compact sparklines.

A map session owns MapLibre readiness, style restoration, route sources, the active
point, inspection, and disposal. Credentials are attached only to the current origin's
basemap proxy paths; the reserved placeholder origin is rewritten to that origin.
Configuration is supplied by the application, and chart presentation does not fetch
it. Pointer inspection yields to vertical touch scrolling and retains keyboard
navigation through repeated GPS samples.

Original tests and goldens are unchanged. The additional `fullReplacementUiCharts`
tests capture fixed desktop and narrow-width DOM/SVG and canvas output, and exercise
pointer arbitration, cleanup, and rich-chart pinch/reset behavior. Static renderer
markup and fixed formatting or geometry equations are explicit compatibility
contracts, rather than an alternative rendering implementation.
