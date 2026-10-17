# Dashboard core implementation

Dashboard rendering uses committed data and the supplied saved configuration.
Layouts are fixed in the UI: legacy `mode="edit"`, `onConfigChange`, and
`editActions` inputs remain accepted during caller migration but have no effect.
The shared renderer never replaces a custom dashboard with a bundled default.
The view markup, bundled chart definitions, query routes, and permission rules
remain compatibility contracts.

The widget registry stores each definition with its resolved editor metadata in
one entry. Registration replaces both together. Fixed-size metadata disables
resizing; existing registration order and default metadata remain unchanged.

View widgets share an intersection observer for deferred mounting. Each boundary
releases its observation after first intersection or unmount; the observer closes
when no boundaries remain. The 320px preload margin and mobile heights remain
unchanged.

`DashboardRenderer` derives visibility and data requirements only from live
vehicle state. It removes a caller's legacy `ctx.visibilityState` override
before passing context to data providers or widgets, while preserving chart
preferences and automatic widget height callbacks. Hidden widgets remain in the supplied configuration; the existing
view projection and responsive grid determine what is displayed. `WidgetChrome`
always renders its view frame and ignores its former editor callbacks. There is
no lazy grid editor, drag or resize overlay, palette, widget form, editor drawer,
or scenario preview. Chart definition editing and favorites remain separate
features. `IconPicker`, daily sensor aggregation, and icon migration remain in
use outside the removed editor chain.

Dashboard API writes share cache reconciliation. A system row cannot replace a
personal row in the slug cache. List entries are reconciled by identity and by
slug/owner, with defaults sorted first. Restore retains its successful dashboard
response if saving the managed chart preference fails; the current session then
uses that restored chart selection.

Chart manager entries are assembled by slug while resolving system and personal
rows independently. A disabled personal override remains authoritative. Bundled
renderer validation resolves bindings once and applies ordered contract checks.
The bundled legacy renderer remains the production route; the generic renderer
registry continues to serve independent chart definitions.

Catalog compilation validates each bundled configuration before deriving its
primary-series mode, axis, range, and recognized placements. The sorted catalog
and ID index share the resulting descriptors. Fixed renderer routing and
capability vocabularies remain explicit; the established chart renderer output
is unchanged. Settings capabilities compose three independent policies: source
filter support, X-domain ownership, and ordered axis descriptors. Axis units
either come from a fixed renderer contract or the definition's primary unit.
Routing, legacy mileage fingerprints, and legacy icon names use maps containing
only their declared keys. Object prototype names cannot select a route,
trigger mileage normalization, or resolve to a non-string icon value.
Legacy trip-stat normalization indexes metric IDs separately from stat aliases,
preserves metric precedence, and leaves unknown aliases untouched.

`WidgetHost` resolves the registry component, composes its optional keyed
presentation, then assembles title/content slots and frame attributes. Title
ownership is separate from those fixed DOM slots. Unknown widgets retain their
fallback frame and do not invoke a presentation. The generic renderer registry
contains one universally supported definition renderer, so resolution returns
that adapter directly; bundled legacy chart dispatch remains separate.

Placement reconciliation preserves unaffected placement objects, checks ordering
while removing the target, and inserts a replacement after locale-equivalent
keys. Portable chart import counts UTF-8 bytes incrementally and rejects
an oversized prefix before allocating a parsed document or a full encoded copy.

Chart validation visits nested parameter objects with an explicit stack and
preserves diagnostic paths and order. Executable, remote, SQL, HTML, and CSS keys
remain prohibited. Source validation checks filter operators, field roles, and
supported marks without expanding access to sources.

Daily sensor data is grouped before projection. Totals sum valid events; counter
deltas select the last lexical timestamp within each UTC day and floor resets
at zero. Padding retains the last observed day as the end of the requested
window. Input arrays are never sorted or mutated.

Icon search uses
abort-scoped requests through the existing proxy and ignores canceled results.
The icon grid decodes labels when its list changes and uses one selection
handler for button, icon, and label clicks. Grid whitespace cannot select an
icon. Iconify's direct fetch path remains disabled.

Existing shared query planning, vehicle placeholder isolation, metric projection,
layout bounds, and bounded expression evaluation remain in place. Local
replacement tests supplement the retained assertions. Tests of removed layout
editing capabilities are retired with complete originals and SHA256 hashes in
the external review record; unrelated view and security assertions remain.
Integration validation must still exercise the assembled dashboard and canonical
chart renderers after the app owner removes route and settings entry points.
