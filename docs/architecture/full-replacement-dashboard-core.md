# Dashboard core implementation

Dashboard rendering separates committed data, temporary editor state, and saved
configuration. The component markup, bundled chart definitions, query routes,
and permission rules remain compatibility contracts.

The widget registry stores each definition with its resolved editor metadata in
one entry. Registration replaces both together. Fixed-size metadata disables
resizing; existing registration order and default metadata remain unchanged.

View widgets share an intersection observer for deferred mounting. Each boundary
releases its observation after first intersection or unmount; the observer closes
when no boundaries remain. The 320px preload margin and mobile heights remain
unchanged.

The grid editor applies add, remove, replace, and layout commands through one
configuration boundary. Every emitted widget passes the registry layout
constraints. A selection belongs to a dashboard identity and clears when that
identity changes or the selected widget disappears from the preview. Preview
state uses explicit synchronization and selection actions: live status updates
an untouched preview, while user selection persists until the dashboard or mode
changes.

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

Chart validation visits nested parameter objects with an explicit stack and
preserves diagnostic paths and order. Executable, remote, SQL, HTML, and CSS keys
remain prohibited. Source validation checks filter operators, field roles, and
supported marks without expanding access to sources.

Daily sensor data is grouped before projection. Totals sum valid events; counter
deltas select the last lexical timestamp within each UTC day and floor resets
at zero. Padding retains the last observed day as the end of the requested
window. Input arrays are never sorted or mutated.

The widget form resolves sensor text options from saved values, definition
values, and explicit defaults. Its raw JSON editor tracks synchronization, text,
and error transitions separately from saved widget options. Icon search uses
abort-scoped requests through the existing proxy and ignores canceled results.
Iconify's direct fetch path remains disabled.

Existing shared query planning, vehicle placeholder isolation, metric projection,
layout bounds, and bounded expression evaluation remain in place. Local
replacement tests supplement the original assertions; integration validation
must still exercise the assembled dashboard and canonical chart renderers.
