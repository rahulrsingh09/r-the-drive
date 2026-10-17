# Saved dashboard state

Dashboard pages render the selected saved configuration. The interface has no
layout-editing session, local layout draft, save/discard flow, or drag/resize
canvas. This applies to built-in dashboards and existing personal dashboards,
including administrator routes and old edit query parameters.

## Identity and refresh

Saved rows retain their exact ID and slug. Normal route resolution prefers the
current user's same-slug personal dashboard over the system row. A request for
an exact dashboard ID must show that row or an inline error; it must not silently
substitute a different same-slug dashboard.

Background data refreshes update the displayed saved configuration without
inventing layout edits or replacing another dashboard's identity. Existing
widgets, positions, titles, options, and conditional visibility remain stored.
Removing editing controls does not authorize an upgrade to reset personal data.

Settings > Dashboards supports opening and exporting saved rows. Layout creation,
editing, import, duplication, reset, deletion, restore, and lock controls are
absent. Persistence APIs and compatibility helpers retain their own server
permissions; UI removal alone does not change those API contracts.

## Layout and visibility

Saved configuration validation preserves schema and geometry constraints.
Rendering may project responsive geometry and live visibility without writing
those view-only changes back into storage.

Visibility rules use AND semantics. A nonempty typed rule list takes precedence
over legacy `chargingConnectionVisibility` options. Compatibility normalization
preserves unrelated options. Plugged and unplugged visibility follows the live
vehicle state; no editable scenario-preview state is exposed.

## Chart preferences

Chart selection and temporary display controls remain view state. A favorite
star persists the selected chart through account preferences. Settings > Charts
continues to own chart definition editing and validation. Neither workflow
creates a dashboard layout draft or exposes dashboard-assignment editing.

## Verification contract

Desktop and mobile checks must cover existing saved geometry, conditional
visibility, chart favorites, and the absence of layout controls. Old edit URLs
and saved edit-button preferences must not enable editing. The removed editing
behavior is an intentional product change; unrelated chart, account, data, and
security assertions remain in force.
