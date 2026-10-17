# Web shell implementation boundaries

The web shell owns route registration, session gating, login and account activation,
root preferences, query persistence, release checks, and dashboard page composition.
The implementation replacement preserves the existing R:The Drive appearance,
public interfaces, and GPL attribution. It does not imply independent or clean-room
origins.

## Routes and account entry

Route definitions retain their paths, parent callbacks, and guard placement.
The dashboard route accepts legacy `edit` search input but discards its value;
`dashboardId` still requires a UUID. Login and activation render from `components/auth`; their route modules
re-export the existing page names. The shared form reducer keeps passwords in
component memory and models pending methods and errors. Login retains its single
loading indicator; activation tracks password and SSO operations independently.

`useLoginSession` owns one refresh attempt and one automatic SSO attempt per mounted
login page. It consumes explicit logout intent once, honors callback and
password-change suppression, and prevents navigation after unmount.
`authSessionLifecycle` coordinates protected-page cache hydration, refresh recovery,
expiry cleanup, and one redirect. Recovery remains coalesced while an expired
session is being resumed.

## Dashboard state

`DashboardPageShell` renders saved layouts in view mode. It has no layout draft,
save/discard action, resize callback, or dashboard mutation hook. Legacy controlled
edit props cannot activate editing. The shell passes the selected configuration
object directly to `DashboardRenderer`; it does not reset, merge, or migrate saved
widget positions. Vehicle selection, date range, units, filters, loading states,
and exact-dashboard errors retain their existing behavior.

`dashboardShellModel` resolves exact dashboard identity separately from optional
bundled fallback. An unavailable exact dashboard never silently opens a bundled
layout. The user-dashboard route retains YAML export and removes clone/import
actions. The admin dashboard list opens a saved dashboard by its ID and exposes
no edit or lock action. `/d/dashboard` still redirects to Overview unless an exact
dashboard ID is supplied, including when legacy edit search input is present.

The old edit session, draft hook, save dispatcher, and dashboard edit-preference
hook have been removed. Stored legacy edit preferences do not enable controls
or trigger storage writes.

The phantom-drain table has a reducer for search, page size, page navigation, and
source changes. Its summary accumulator maintains independent duration weights for
sleep and coverage. Parked-energy presentation indexes the first sample of each
window and preserves the fresh-since-parked, 24-hour, then first-sample preference.
The bundled chart widget and its fixed instance contract remain unchanged.

## Browser state and request boundaries

Query persistence uses a delay/idle scheduler, a size limit, the existing successful
query allowlist, and per-user storage keys. Auth, errors, and explicitly private
queries are not persisted. Session clearing still cancels queries and clears the
cache.

Release checks separate the global in-flight attempt from each mounted scheduler.
Disabled settings and invalid running versions do not start a schedule. Hidden
documents do not initiate checks. Requests retain omitted credentials, no referrer,
no HTTP cache, and the 15-second abort deadline. Failed storage writes retain the
current tab's attempted timestamp until a newer durable result is available.

Root preferences preserve session and account query gates. Theme mode writes retain
custom theme definitions and revisions, require the cached ETag, and ignore
completion for a different authenticated owner.

The timeframe codec accepts current and legacy stored shapes. Typed numeric date
input selects a date-fns grammar from its year and clock form; native date parsing
remains the fallback. Presets preserve app-calendar-day boundaries and hour-based
ranges.

## Validation

Original tests and goldens unrelated to the removed layout capability remain
unchanged. The complete prior capability test files and SHA256 hashes are archived
in the recovery review ledger before their narrow updates. Additional tests prefixed
`fullReplacementWebShell` exercise cache privacy, request deadlines, storage
failure handling, saved configuration identity, session lifecycle, redirect
canonicalization, password progress, efficiency filters, and notification expiry.
The browser editability suite checks fixed widget placement and the absence of
dashboard writes on desktop and mobile, including manual edit queries and admin
entry points. Integration with other frontend scopes and final application-wide
mobile acceptance remain coordinator responsibilities.

## Documentation integration

Documentation impact: both internal and user-facing docs required. This page
records the shell behavior. The integrating owner must register it in the shared
sidebar and update `docs/branding.md`, `docs/frontend/dashboard-architecture.md`,
`docs/frontend/dashboard-authoring.md`, and user-facing dashboard guidance to
describe the combined removal across the shell, Settings, R tools, and renderer.
Those shared documents are outside this leaf's ownership.
