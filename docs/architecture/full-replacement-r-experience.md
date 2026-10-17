# R Experience implementation replacement

The R Experience presents the existing dashboard, telemetry and navigation contracts. This change replaces its inherited controllers and derivations while keeping the established markup, CSS, artwork, chart renderer, public component signatures and interactions. It does not claim independent or clean-room authorship. Existing licensing and attribution remain applicable.

Documentation impact: internal architecture update. Shared documentation indices remain owned by the integration coordinator.

Integration follow-up: the coordinator must register `architecture/full-replacement-r-experience` exactly once in `apps/docs/sidebars.ts`. The scoped worktree intentionally leaves that shared file unchanged; `docs:check` reports the missing registration until integration.

## Data and presentation

`telemetryModel` admits a reported field only when its availability permits use. Numeric and lock readings share that admission boundary. Query records must match the selected vehicle and must not be placeholder data. `connectionModel` applies that boundary to snapshots and live data, and maintains the existing vehicle/issue toast cooldown through the shared feed-health helper. Feed and renewal notices continue to derive only from the admitted snapshot.

`vehiclePresentation` computes tire completeness, extremes and warning state in one accumulator, then resolves charging and software display states. `stagePresentation` prepares identity, artwork and formatted measurements. `activityPresentation` selects the established loading/error priority before formatting visible history rows. The React components retain their fixed markup and text-node boundaries.

`dashboardModel` partitions the home dashboard and rebases each section without mutating the source configuration. Bundled-layout admission indexes canonical widget signatures while preserving the original first-ID-match behavior. Widget options and titles remain outside this geometry comparison. All chart rendering continues through the existing `DashboardRenderer`; bundled charts continue to use its canonical legacy renderer.

Import state uses explicit begin, loaded, failed and dismiss transitions. Creating and cloning dashboards share an outcome boundary and the same destination contract. Import still clears the draft after creation and before navigation; a navigation failure therefore appears outside the closed dialog. Mutations, errors, pending controls and permissions retain their existing behavior.

`settingsModel` indexes visible sections into ordered groups, preserving the known groups, extra-section ordering, search text, and “Units & time” label. `browserBindings` owns the viewport subscription and document input-mode listener lifecycle. Palette extraction reads both fixed CSS scopes and disposes its probes.

## Orbit lifecycle

`OrbitFrameStore` owns decoded resources, in-flight requests and recency timestamps. The immutable packed asset is fetched once per module lifetime, failed pack loads remain retryable, concurrent requests for a frame share a decode, and at most six decoded bitmaps remain retained after each decode. Disposal closes retained and late-arriving images.

`OrbitController` owns viewport entry, reduced-motion changes, visibility, animation, pointer capture, keyboard commands and painting. React owns the rendered view state and connects events to the controller. A single consumer paints the current request and then the latest queued request. The controller preserves the 72-frame wrapping, 56 default frame, 65 front frame, cubic easing, original durations, six-pixel drag threshold, vertical-scroll cancellation, bounded inertia and per-vehicle remembered angle. Disposal disconnects observers, cancels animation and closes the cache.

## Precisely retained fixed contracts

The following small components remain byte-identical because their implementations consist of fixed presentation contracts rather than replaceable state machines. Their complete bodies, hashes, conditions and individual dispositions are recorded in the scope review inventory for coordinator inspection.

| Component | Exact retained contract |
| --- | --- |
| `RMark` | One `span.r-mark`, `aria-hidden="true"`, containing `R`. |
| `RAuthBrand` | One `div.r-auth-brand` containing `RMark` and a paragraph containing `R:The Drive`. |
| `ROverview` | One `RHomeWorkspace` binding `state`, `actions`, and `RRecentActivity` with `state.ctx`; the overview CSS import is fixed. |
| `RHealthHero` | Fixed three-part identity/artwork/summary markup; model fallback, optional VIN, authenticated artwork and children retain their exact bindings. |
| `RLockIcon` | Null selects the fixed help icon; booleans select the existing SVG with the unchanged shackle, rectangle and keyhole paths and attributes. |
| `RConfiguredVehicleArtwork` | The replaced `isConfiguredR2` predicate selects the exact lazy hero image and alt text, otherwise the supplied children fragment. |
| `RBatteryNavigation` | Only `/battery` and `/battery/phantom-drain` render the two unchanged exact-match links. No prefix/trailing-slash admission is added. |
| `RMapPreferences` | Only `openfreemap` renders `AppearanceSettings`; absent map preference binds `follow-theme`. No alternative map provider is enabled. |
| `RDashboardSurface` | Ready, non-editing `/` renders the overview; otherwise the fixed surface binds dashboard slug, non-editing bundled-layout admission, battery navigation and children. |

Two scalar primitives also retain their exact bodies: `finiteReading` is the type/finite gate returning the original number or null, and `wrapFrame` rounds then applies positive modulo 72. Static navigation metadata, settings descriptions, frame constants, the widget-presentation registration and heading context remain contracts. Tiny state selectors, event argument bindings and pure JSX row templates are individually identified in the inventory; retaining those does not exempt their surrounding controller.

## Validation boundary

The uniquely named `fullReplacementRExperience` tests characterize the unchanged source and run against the replacement. Original tests and baseline snapshots are preserved. Coverage includes telemetry admission, role visibility, selected-vehicle isolation, palette extraction, import/clone outcomes, section geometry, history priorities, pointer/keyboard orbit controls, pack failures, LRU behavior, late decode disposal, document cleanup and notification throttling.

Local browser comparisons cover static characterized component fixtures and a live bundled orbit at desktop and mobile sizes, in both light and dark modes. Browser traffic is restricted to an isolated loopback server; no live account, provider, map tile, vehicle or backend is used. Full application navigation, real service integration and final integrated mobile acceptance remain the coordinator's checks.
