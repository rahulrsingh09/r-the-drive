# Web shell implementation boundaries

The web shell owns route registration, session gating, login and account activation,
root preferences, query persistence, release checks, and dashboard page composition.
The implementation replacement preserves the existing R:The Drive appearance,
public interfaces, and GPL attribution. It does not imply independent or clean-room
origins.

## Routes and account entry

Route definitions retain their paths, search schemas, parent callbacks, and guard
placement. Login and activation render from `components/auth`; their route modules
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

`dashboardEditSession` and `useDashboardEditDraft` remain the existing replacement
state machine. They reconcile saved configuration before children commit and retain
dirty drafts across refreshes. `dashboardShellModel` resolves exact dashboard
identity separately from optional bundled fallback and produces immutable widget
height edits.

`dashboardSave` chooses a system update, an owned-copy update, or initial creation.
Only a failed creation enters the refetch-and-update recovery path. Saved identity
and permissions come from the selected target, while editable content comes from
the draft. Save failures keep the draft open.

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
completion for a different authenticated owner. Dashboard edit-button preferences
subscribe directly to per-account browser storage.

The timeframe codec accepts current and legacy stored shapes. Typed numeric date
input selects a date-fns grammar from its year and clock form; native date parsing
remains the fallback. Presets preserve app-calendar-day boundaries and hour-based
ranges.

## Validation

Original tests and goldens remain unchanged. Additional tests prefixed
`fullReplacementWebShell` exercise cache privacy, request deadlines, storage
failure handling, draft resolution, session lifecycle, account isolation, and
notification expiry. Static JSX contracts and loopback browser checks accompany
the focused route tests. Integration with other frontend scopes and final
application-wide mobile acceptance remain coordinator responsibilities.
