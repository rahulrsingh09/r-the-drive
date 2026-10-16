# R experience integration

## Ownership

This private fork exposes R (the approved Option B) as its only interface.
There is no interface switch or alternate application layout. R compiles into
the existing web app and uses its API and shared packages.

`apps/web/src/features/r-experience` owns composition, navigation, settings
discovery, the vehicle stage, motion and scoped CSS. Existing hooks own auth,
account preferences, vehicle selection, cache keys and API transport. Existing
dashboard/settings controllers retain editing, permissions, queries and mutations.
The canonical bundled-chart renderer is unchanged. Only analytics matching the
bundled widget geometry use compact R spacing. Home groups its saved widgets
into the responsive sections described below; custom dashboard pages and editing
mode retain saved positions and sizes.

| Shared seam | Purpose |
| --- | --- |
| `routes/__root.tsx` | Apply R across routes, authentication and portals |
| `components/layout/AppLayout.tsx` | Export R under the existing layout contract |
| `components/layout/vehicleConnection.ts` | Feed-health and connection-state helpers used by R |
| `components/dashboard/DashboardPageShell.tsx` | Pass existing state/actions to R composition |
| `components/dashboard/dashboardShellTypes.ts` | Shared types without a module/controller cycle |
| `features/settings/SettingsPage.tsx` | R directory around existing sections and authority |
| Health and chart editor composition | R identity while preserving workflows |
| `routes/settings.themes.$themeId.tsx` | Redirect retired appearance URLs |
| Card, PageLayout, MetricTabs and WidgetHost attributes | Stable styling hooks |
| `WidgetPresentationProvider` | Scoped vehicle presentation without changing widget definitions, data requirements, saved configuration or canonical charts |
| `packages/ui/src/tokens/globals.css` | R overrides in the canonical token owner |
| `DataPaletteProvider` and `useChartColorPairs` | Fixed R data colors without changing stored account themes or explicit chart colors |
| TripMapChart and trip-detail composition | Optional point inspection; proxy policy unchanged |
| SelectPicker | Shared viewport placement and keyboard dismissal across settings and dashboard menus |

The mainline React UI and Rust security modules remain the application baseline.
Authentication, vehicle authorization, key custody, gateway checks and history
protection retain their existing integrations. Supporting auth and chart
controllers are active dependencies; they are not alternate UI or security
implementations. Changes to these boundaries must preserve their contracts.

The overview uses actual existing metrics, trips, charging and vehicle status.
Selected-vehicle identity and placeholder checks prevent displaying another
vehicle's cached readings. Missing readings remain unknown, not zero. Home
incorporates the chart workspace and dashboard tools; `/d/dashboard` redirects
to `/` unless an exact dashboard ID or editing query was requested.
The R2 vehicle widget owns the interactive stage and compact status summaries.
Its poster and rotation canvas share a uniform 1.1 scale and a theme-token
radial backdrop. This closer, soft-light treatment adds no assets or requests.
The car precedes period trip distance, total trips and efficiency cards; SOC,
estimated range and charging use a matching second card row. Linked drive, climate,
tire and software summaries open Vehicle health. A single vehicle lock icon
reflects the overall `doors_locked` reading, with an accessible label and hover
title. Missing or unavailable readings show an unknown icon. The shackle follows
known lock changes and respects reduced motion. It is not a button or a vehicle
command; there is no door/frunk/gate modal, open count or individual indicator.
The presentation provider also applies inside dashboard copies and editing
previews; other models retain their model-specific artwork.

The main car uses the unchanged photographic rotation. Opening outlines,
cutout effects and recorded opening clips are intentionally omitted. Do not
infer physical opening or overall lock state from individual closure fields.

Assigned chart catalogs initialize their selection after saved favorites load.
Without a saved favorite, the configured chart is selected and starred by
default. Changing that favorite leaves independently pinned graph cards intact.

Home groups the saved configuration into the first vehicle widget, sensor
totals and remaining analysis widgets. Each section uses `DashboardRenderer`;
the original configuration remains the source for editing, export and writes.
The vehicle occupies the left desktop column. Recent activity and a two-column
grid of totals share the right column; charts span the row below both.
Without a vehicle widget, totals retain their full-width layout.
Mobile orders vehicle/readings, totals,
recent trips/charging, then charts. Relative widget rows are normalized within
each section without mutating saved geometry. No vehicle is injected if absent.
The first vehicle widget supplies the page heading through app-local context;
removing it leaves a Home heading and dashboard tools. Visibility rules, addition,
removal and widget options remain owned by the shared renderer. Home's vehicle
section uses content height, including for previously shortened widgets.
Other custom dashboard pages and the editor retain saved dimensions and ordering.
The bundled distance sensor uses `trip_miles` as total trip distance instead of
cumulative `total_miles`. View, export and editing use the same saved config;
there is no additional Home-only metric projection. Import validates existing YAML and
creates a uniquely named personal copy through the existing dashboard API after
the user confirms. Unit subscriptions retain the full preference object so
changing efficiency display updates cards, charts and tables without a reload.

R fixes all series, status, map and drive-mode tokens in both appearance modes.
Its data palette provider selects categorical sensor slots consistently and reads
custom-picker defaults from those same CSS tokens. Stored Classic, RAD and custom
account themes remain compatible but do not select ordinary R chart colors.
Explicit colors saved on individual charts remain intentional overrides.

The daily charging SVG renderer measures its content width before laying out
labels. Phone chart cards reserve space for controls and axes without shrinking
desktop typography. Selected-trip tag controls wrap into a full-width picker and
a separate labeled action row; their existing mutations and permissions are
unchanged.

## Routes and settings

Desktop primary navigation has Home, Trips, Charging, Battery, Efficiency
and Explore. Mobile uses five equally sized, labeled tabs: Home, Trips,
Charging, Battery and Explore. Efficiency is reached from the Home summary or
Explore and keeps Explore selected on mobile. Resizing restores its direct
desktop tab without changing the route.
Battery overview and Phantom drain share a visible section navigation, and both
keep Battery selected in the primary bar. Exact section-route matching prevents
the overview link from remaining active on Phantom drain. Explore retains direct Battery and
Phantom drain links alongside Vehicle health, dashboard library, settings and
permission-gated account administration.

System dashboard baseline 9 adds nine pinned Home charts, five Battery charts,
four Charging charts and four Efficiency charts. The existing catalog remains
for additional assigned charts. Every original Overview chart type is pinned on
at least one default page. Pinned widgets use a single assigned chart slug with
`showPicker: false`; they keep the effective chart's saved definition and canonical
renderer, ignore favorites for other slugs, and never substitute another chart
when the requested one is unavailable. They are normal, removable widgets, not
managed composition injected into personal layouts.

The canonical JSON defaults and API mirrors remain synchronized. On API startup,
the existing baseline mechanism updates system defaults once; this also replaces
previous administrator edits to those system defaults. Personal dashboards are
not replaced or populated with the new pinned charts. Their existing managed
catalog migration is unchanged. Restore system default uses the same new baseline.
Chart requests remain deferred until close to the viewport and use the existing
query cache; no additional polling, collector or data service is enabled.

Settings retain Vehicles, Dashboards, Charts, Units & time, Places, Charging,
External connections, API access, Jobs, Raw data, Backups, Account and
Authentication. Existing roles determine availability. Map controls move to
Units & time. `?section=appearance` opens Units & time; `/settings/themes/:id`
redirects to the directory. Stored theme records and light/dark preference remain
compatible with upstream, but R overrides the interface palette and exposes no
theme chooser.

The R route-coverage contract enumerates every registered route, including auth,
focused editors and compatibility URLs. An upstream route addition requires a
coverage decision. Browser tests exercise the normal shell, focused chart
editing, map inspection and authentication separately.

## Artwork and motion

The owner's R2 Performance presentation is fixed to Catalina Cove, 21-inch wheels
and black interior, including when live paint or wheel metadata is missing.
This is an explicit owner configuration, not inferred telemetry. Other vehicles use the existing authenticated
artwork resolver and fallback. Artwork is presentation, not a live sensor reading.
Health and Settings → Vehicles share `RConfiguredVehicleArtwork`; Overview uses
the same image while its rotation frames load.

The poster is approximately 153 KB. The 72-frame rotation pack is 3,460,102 bytes,
fetched once per document when the visible car begins its first automatic turn,
on interaction, or on restoration of an angle already selected in that document.
The entrance turn runs once for 3.6 seconds and yields immediately to interaction.
Six decoded frames are retained per mounted viewer.
Images and animation frames are released on exit; fast drags draw the latest
requested frame. Failed loads stop motion until another explicit attempt.

There is no CDN, remote model request or new animation dependency. Assets use
the existing app origin/security policy. Reduced motion removes the automatic
turn and inertia, deferring the frame pack until direct interaction. Leaving the
viewport, hiding the tab or unmounting stops motion.

## Security and cost boundaries

Home disables the shared shell's edit shortcut and omits the dashboard-tools
edit action. The Settings dashboard library still opens the canonical editor.
Direct edit routes wait for the saved API configuration before creating a draft;
they must not capture the bundled fallback while a saved layout is loading.

Phantom Drain opts into the shared table's responsive row renderer. Mobile cards
use the same sorted row model and page data as the desktop table; desktop column
controls remain intact. Canonical charts still own drain aggregation. Shared
rich charts measure their real width and legend height, and enable touch-only
scrubbing on embedded plots without changing desktop zoom or fullscreen gestures.

The sign-in form reads named fields at submission so browser-filled credentials
cannot diverge from older React state. Password bytes are preserved exactly.
Cloudflare Access and the app's existing password verification remain separate;
this form correction does not reset accounts or change either access policy.

API private-deployment modules, private gateway, database catalog adapter,
origin gate and release guards do not depend on R. R adds no credentials,
endpoints, permissions, migrations, workers, databases or cloud services.
No free-tier policy exception is added.

Retaining source does not allocate another service. Asset transfer still counts
against existing request/egress usage. Rotation is deferred; authenticated
gateway caching is unchanged, so reloads can transfer assets again. This is not
a claim of unlimited free bandwidth. Actual allocation must pass the unchanged
Northflank `free_policy` and billing checks before deployment; a local build does
not prove future billing will stay zero.

## Upstream synchronization

Use the existing fork-upstream-sync, candidate CI and protected promotion path.
Do not use automatic “prefer ours” merge drivers. The sync gate stops when
upstream touches any fork-modified path, even if Git can merge the text. Module
isolation reduces overlap; shared seams still need review.

1. Fetch upstream and identify the exact candidate; skip integrated commits.
2. Inspect overlap at the UI and security seams.
3. Preserve upstream auth, cache scoping, canonical charts and settings behavior.
4. Run build, package/script tests, browser/route coverage, color/dependency and
   security-route checks, plus unchanged deployment guards.
5. Review desktop/mobile, dark/light and failure-path behavior.
6. Promote only through existing protected CI and verified release.

Deployment is a separate action after review. The existing frontend CI job runs the full browser suite, with no new CI job
or deployment resource. The sync and promotion policy is unchanged.
