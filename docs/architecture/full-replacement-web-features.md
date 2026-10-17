# Web feature controllers

The feature pages preserve the R visual system, chart definitions, favorites, and permission gates. Dashboard layout restructuring is intentionally removed from Settings and chart management. Stateful work is organized around explicit draft transitions and requests bound to the resource that started them. This implementation replacement preserves the project's licensing, attribution, and history.

## Editing and validation

The chart draft reducer owns the structured definition, advanced JSON text, validation failure, dirty flag, and save feedback. Invalid JSON remains visible without replacing the last valid definition. Advanced JSON rejects placement changes; valid chart-definition edits retain the saved placements. Newly created, duplicated, and imported chart definitions start unassigned. Changing an existing chart still preserves its assignments. Typed definition edits apply to an isolated copy. Curve transactions resolve an existing source binding before allocating another, enforce the four-source limit, update the shared domain when required, and remove unreferenced bindings. Both source-capability validation and bundled-renderer validation remain required. Bundled charts continue through `ManagedChartRuntime` and the canonical renderer.

Theme drafts keep the resource ETag and saved revision together. Refetching an older resource cannot move the publication target backward. Account or theme changes create a new editor session. Preview cleanup restores account preferences when the editor leaves. Contrast auditing retains the existing interface thresholds and chart warnings; resource definitions still cross the existing theme-client validation boundary.

Place editing groups the address, draft, and edit target into one transition. Schedule parsing retains validation precedence, midnight boundaries, and full-day coverage. Charge corrections assemble a request only after the selected location and manual cost are valid. Authentication settings derive a field model from server metadata and omit environment-owned fields from updates. Secret replacement and deletion remain explicit, separate intentions.

## Request ownership

Connection credentials and OTP use the same connection state machine. Each asynchronous step checks its request generation before starting the next step or publishing a result. Refresh mode still verifies that the authenticated Rivian account contains the selected existing vehicle. A response after navigation cannot add a vehicle or change the current default.

Settings operations use a local operation generation to reject superseded completion callbacks. Cache invalidation for account membership changes uses the user from the request variables. Charging feedback carries its vehicle identity, and chart import checks ownership after file parsing and before creation. Existing role checks, private-network controls, confirmation prompts, and API error redaction are preserved.

Account editors retain their source identity, so an earlier account's save cannot close the current editor. Invitation dialogs use a generation that changes on both opening and closing; a late activation token cannot appear in a newly opened dialog. Authentication and external-connection forms commit secret replacement, removal, feedback and server-snapshot resets as explicit transitions.

The restore workflow holds the inspected artifact and plan together. Execution still submits the confirmation phrase, plan identifier, and package checksum. The restore observer permits one request at a time, retries temporary API loss, stops on terminal failure, and owns the completion reload timer. Disposal cancels timers and suppresses late responses.

## Detail pages

Trip timeline projection constructs chart points, coordinate-to-sample mappings, and coverage counts from the same source columns. The previously replaced recorded/GPS speed reconstruction is retained. Cursor selection publishes at most once per animation frame and cancels queued work when its subscribers leave. Vehicle health derives diagnostic priority and software installation runs without changing the visual status vocabulary.

## Verification

Unrelated original tests and goldens remain unchanged. Original dashboard-restructuring assertions are archived with SHA-256 hashes in the recovery evidence before narrowly replacing them with absence and preservation checks. Additional `fullReplacementWebFeatures` tests cover draft immutability, invalid JSON, theme revision ownership, environment-owned authentication fields, place schedule boundaries, charge-cost validation, restore polling, and cursor cancellation. Connection cancellation has baseline failure and candidate success evidence. The accompanying scope inventory records each baseline executable unit and the retained fixed DOM, data, and trivial adapter cases for integration review.

Documentation integration remains with the frontend coordinator: register this page in `apps/docs/sidebars.ts`. The feature scope excludes that shared index.

## Fixed saved layouts

Settings → Dashboards lists system defaults and saved personal layouts with Open and Export actions. It does not create, customize, duplicate, import, edit, delete, reset, restore, or unlock layouts, and it does not expose the show-edit preference. The chart manager displays saved dashboard assignments without an assignment editor. None of these changes resets or rewrites saved layout data. Chart definition creation, editing, duplication, JSON import/export, enablement, and existing chart permissions remain available.

Tests cover regular users, admins, super users, a previously enabled show-edit preference, the `edit=1` query, saved-layout opening and reload, and chart JSON assignment attempts at desktop and phone widths. Shared dashboard rendering, route bypass handling, and the other layout entry points belong to the coordinated shell, dashboard-core, and R-experience changes.

Documentation impact: **Both internal and user-facing docs required**. Tracked integration follow-up: the frontend coordinator must register this architecture page in `apps/docs/sidebars.ts` and reconcile the layout-editing descriptions in `docs/branding.md`, `docs/frontend/dashboard-architecture.md`, and `docs/frontend/dashboard-authoring.md` with the combined capability removal. Those shared documents are outside this leaf's ownership.
