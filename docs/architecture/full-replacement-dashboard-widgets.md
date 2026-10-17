# Dashboard widget implementation boundaries

Documentation impact: Internal doc update required. Widget markup, chart families,
routes, query permissions, and schedule feature gates retain their existing contracts.

The widget layer separates event transitions, data preparation, and rendering. Its
public registry entries and component signatures remain compatible with dashboard
definitions.

## Selection and table state

`widgetStore.ts` supplies immutable snapshots to `useSyncExternalStore`. Trip
selection and table pagination use separate reducers. Registering fetched trips
updates the lookup without notifying selection subscribers; explicit selection
events publish the next snapshot. Vehicle and time-range changes reset selection
and pagination through their existing context keys.

`tableProjection.ts` prepares selected rows, common tags, charging-session
readings, map routes, and trip labels. It preserves selection order across pages.
The trip-tag model resolves catalog matches and keyboard actions.
`useTripTagBatch.ts` captures the submitted selection and handles pending,
success, and failure transitions. A failed mutation retains staged input.

## Chart routing and preparation

`DashboardChartWidget.tsx` keeps known bundled slugs on their canonical chart
components. The source registry selects the existing chart family; it does not
choose a generic renderer based on the shape of an edited definition. Only
unknown slugs use the pre-existing generic definition path.

The canonical adapters share display-option and axis preparation.
`chartProjection.ts` joins domains and prepares series while retaining established
colors, labels, and series identities. Session curves, mileage charts, and
efficiency charts have separate projection modules. The charging-analysis
regression builds ordered neighborhoods; historical samples with estimated state
of charge remain excluded from verified summaries.

The mobile viewer owns viewport, fullscreen, orientation, keyboard, and pointer
state through `mobileChartController.ts`. Its dialog still renders the same chart
family and restores the page styles when it closes.

## Sensors and vehicle presentation

Sensor options use a whitelist of nullish defaults so explicit `false`, zero, and
empty-string values survive. A readout selects status, loading, trip selection,
or configured data once. Formula evaluation still delegates to the shared safe
expression evaluator.

Vehicle image selection indexes the supplied image metadata and applies the
existing design and token priorities. An empty URL still consumes its matching
priority slot. Artwork continues through the authenticated artwork components
and hooks. Geometry and fallback models preserve the existing anchors, crops,
and closure overlays.

## Schedule editing

`scheduleState.ts` owns form drafts, editor visibility, and pending operations.
Charging and departure widgets keep their own input contracts and provider
identifiers. Demo read-only behavior and `VEHICLE_CONTROLS_ENABLED` remain at the
existing UI boundaries; this change does not enable vehicle controls.

## Verification

The existing chart compatibility, sensor, table, mobile, vehicle, and schedule
tests remain unchanged. Additional characterization is in
`fullReplacementDashboardWidgets.contract.test.tsx`,
`fullReplacementDashboardWidgets.algorithms.test.ts`, and
`fullReplacementDashboardWidgets.views.test.tsx`, and
`fullReplacementDashboardWidgets.liveNetwork.test.tsx` under `apps/web/src/test`.
Those assertions were run against the baseline before their corresponding
implementation changes. Tests use synthetic query data and local browser
fixtures that reject external requests.

Fixed registry data, public lookup mappings, literal DOM templates, and minimal
public-contract adapters are recorded individually in the accompanying review
inventory. They are not represented as independently authored replacements.

## Recovery review

Chart display settings are independent of dashboard layout editing. The optional
`updateWidgetOptions` callback receives only `chartSettings`; without it, controls
apply to the current preview. The widget no longer directs users to dashboard edit
mode. Existing saved positions, dimensions, chart choices, and display settings
remain inputs to rendering. Dashboard editor removal and route authorization are
owned by the dashboard renderer and web shell, outside this widget boundary.

`chartCapabilities.ts` resolves bundled capabilities before inspecting a custom
chart definition. `scatterProjection.ts` indexes numeric samples into the six
established power buckets and prepares their color ramp once. The regression keeps
its original accumulation order and weighted least-squares expressions; changing
expression grouping would change floating-point results. `weightedRegression.ts`
prepares five ordered moment columns and classifies the fit as absent, constant,
or linear before evaluating it. The determinant, slope, intercept, and clamp
equations remain individually retained; the preparation and solver structure
are replacements.

`tableControls.ts` shares pagination boundary rules and builds distinct clear,
add, and replace tag commands. `schedulePresentation.ts` prepares schedule display
fields and comfort pills in their established order. `sensorReadings.ts` shares
finite-sample selection between metric fallback eligibility and sprite rendering.
Recovery checks also cover live zero readings, absent selected rows, numeric edge
cases, widget display controls under an edit query, and saved widget geometry.

The final residual review replaces the overview and charging widgets' repeated
vehicle selection with `useWidgetVehicle`. Its catalog index keeps the first
matching vehicle and preserves explicit demo flags ahead of legacy identity
prefixes. Overhead artwork is rendered from a layer plan: fallback reports change
the active layers independently for each theme, suppressing door overlays only
while that theme uses fallback art. Existing coordinates and fallback transforms
are unchanged.

Efficiency tag sources derive an explicit error, onboarding, or chart state.
Catalog errors take precedence over stale data, and an empty loaded catalog takes
precedence over efficiency loading. Management permission affects onboarding
copy. The tag picker's effect owns focus on opening and desktop dismissal
subscriptions together; a breakpoint change does not reset its active option.
`fullReplacementDashboardWidgets.residual.test.tsx` characterizes these controls,
and `fullReplacementDashboardWidgets.weightedRegression.test.ts` checks the
numeric fit classifications and accumulation behavior.

Integration follow-up: the root owner must register
`architecture/full-replacement-dashboard-widgets` exactly once in
`apps/docs/sidebars.ts`, then rerun `pnpm docs:check`. That shared sidebar is outside
the widget worktree's assigned edit scope.
