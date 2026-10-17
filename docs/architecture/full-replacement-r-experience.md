# R Experience implementation replacement

The R Experience presents the dashboard, telemetry and navigation contracts. Its controllers and derivations are replaced while the established CSS, artwork, chart renderer and saved-layout geometry remain intact. Layout restructuring is intentionally removed. This work does not claim independent or clean-room authorship; existing licensing and attribution remain applicable.

Documentation impact: internal architecture update. Shared documentation indices and branding guidance remain owned by the integration coordinator.

Integration follow-ups:

- Register `architecture/full-replacement-r-experience` exactly once in `apps/docs/sidebars.ts`; the scoped worktree's `docs:check` reports the missing registration until integration.
- Update `docs/branding.md` and the user-facing dashboard guidance to describe fixed layouts with Export retained. These shared-document changes belong to the coordinator.
- Run the R browser bypass checks after integrating the shell, renderer and settings owners. This leaf does not change their routes or editing components.

## Data and presentation

`telemetryModel` admits reported fields only when availability permits use. Numeric and lock readings share that boundary. Query records must match the selected vehicle and must not be placeholder data. `connectionModel` applies that boundary to snapshots and live data and maintains the existing vehicle/issue toast cooldown. Feed and renewal notices continue to derive only from the admitted snapshot.

`vehiclePresentation` computes tire completeness, extremes and warnings in one accumulator, then resolves charging and software states. `stagePresentation` prepares identity, artwork and formatted measurements. `configuredVehicleIdentity` supplies the configured R2 identity to both the stage caption and artwork description. `activityPresentation` selects loading/error priority before formatting history rows. The React components preserve markup and text-node boundaries.

`dashboardModel` partitions the home dashboard and rebases each section without mutating its source configuration. Bundled-layout admission indexes canonical widget signatures while preserving first-ID-match behavior. Widget options and titles remain outside that geometry comparison. All chart rendering continues through `DashboardRenderer`, with the canonical legacy renderer for bundled charts.

## Fixed layouts

`RDashboardTools` retains Export. Clone controls, YAML import, the import dialog and its mutation/navigation machinery are removed. Its Dashboard library link and the Explore library shortcut are removed. The R settings directory, search and section picker retain the read-only Dashboards section, which opens and exports saved layouts. Charts, units, data imports and other settings remain available.

`RDashboardSurface` chooses the home workspace using route and readiness. It does not consult edit flags. A stored layout retains its widget IDs, options, ordering and positions; the existing home section rebasing and responsive renderer behavior remain in use. Removing editing does not reset or write stored configurations.

`navigationModel` computes primary navigation admission and Explore directory entries before rendering. Battery route eligibility and its links share one route table. Explore still admits People & access only for `admin` and `super_user`. Theme mode, icon and command share one table. Lock accessibility and icon dispatch share the existing three-state presentation. Map settings dispatch uses the explicit `openfreemap` provider registration; no other provider gains settings.

`settingsModel` indexes available sections into ordered groups, preserving extra-section ordering, search text and the “Units & time” label. Settings selection resolves an offered section before invoking its callback. `browserBindings` owns viewport and document-input lifetimes. Palette extraction reads both fixed CSS scopes and disposes its probes.

## Orbit lifecycle

`OrbitFrameStore` owns decoded resources, pending requests and recency timestamps. The packed asset is fetched once per module lifetime, failed loads remain retryable, concurrent requests for a frame share a decode, and at most six decoded bitmaps remain retained after each decode. Disposal closes retained and late-arriving images.

`OrbitController` owns viewport entry, reduced-motion changes, visibility, animation, pointer capture, keyboard commands and painting. React owns the rendered snapshot and connects events to the controller. One consumer paints the current request followed by the latest queued request. It preserves 72-frame wrapping, the default and front angles, easing, durations, the six-pixel drag threshold, vertical-scroll cancellation, bounded inertia and per-vehicle remembered angles. Disposal disconnects observers, cancels animation and closes the cache.

## Individually retained expressions and adapters

All 30 cases from the first handoff are reviewed individually in the recovery inventory. Conditional route/capability selection is not exempted merely because it appears in JSX. The following remaining bodies have no undisclosed controller:

- `RMark`: the literal `R` span and its fixed accessibility attribute.
- `RAuthBrand`: the mark and literal `R:The Drive` paragraph.
- `ROverview`: direct bindings to `RHomeWorkspace` and `RRecentActivity`.
- `RHealthHero`: three fixed identity/art/summary slots, with `model || 'Vehicle identity pending telemetry'` and `vin && <small>VIN {vin}</small>`. These exact scalar fallback/presence expressions and the fixed authenticated-artwork adapter remain disclosed; no substantive replacement is claimed for them.
- `finiteReading`: the numeric type/finite gate returning the original number or null, including preservation of negative zero.
- `wrapFrame`: round, then positive modulo 72; signed and repeated frames keep the same result.
- Auth selectors, first-match vehicle ID equality, controlled search input forwarding, settings group-ID projection, refetch adapters and logout-before-navigation remain exact, individually recorded bindings. The lock SVG geometry is fixed data; its selection now comes from shared lock presentation.

## Validation boundary

The `fullReplacementRExperience` tests cover retained telemetry, role visibility, selected-vehicle isolation, palette, section geometry, history priorities, orbit input, decode/disposal, document cleanup and throttling. Additional tests cover absence of clone/import/library shortcuts, stale edit inputs, read-only saved-layout navigation and saved custom geometry. Retired capability assertions and the original two affected snapshots are archived with hashes; only the removed controls are trimmed from those snapshots. Unrelated golden nodes and the original six feature-test files remain intact.

The R browser tests exercise the actual application with synthetic API fixtures, canonical charts, saved layouts and desktop/mobile sizes. Saved-layout cases start with read-only fixture data and deny writes. The new integration bypass cases cover user, admin and super-user roles, stale edit preferences and direct URLs. They must run against the combined fixed-layout shell/settings/renderer implementation before acceptance. Local browser execution uses a task-owned container with no external network. No live account, vehicle, provider or backend is used.
