# Dashboard session state

Dashboard editing separates the latest saved configuration from the active
draft. `useDashboardEditDraft` retains the public hook interface used by
`DashboardPageShell`; its state transitions live in the pure
`dashboardEditSessionReducer`.

## Session transitions

The session records the exact dashboard identity, effective edit mode, latest
saved data, draft, and save error. Identity includes both the dashboard ID and
slug. Before configuration data arrives, the identity is `pending:<slug>`.
Two personal copies with the same slug therefore have separate edit sessions.

A draft is empty, pristine, or dirty. Pristine means its contents match the
saved configuration. Equality retains the existing serialized configuration
comparison, including nested widget options.

| Event | Result |
| --- | --- |
| Enter edit | Initialize from saved data and clear the save error. |
| Exit edit | Discard the draft and clear the save error. |
| Edit configuration | Apply the replacement or functional update, classify the draft, and clear the save error. |
| Save error | Retain the draft and update only the error. |
| Change exact identity or effective edit mode | Initialize a new session for that identity and mode. |
| Refresh saved data with a pristine draft | Adopt the refreshed configuration and remain pristine. |
| Refresh saved data with a dirty draft | Retain local contents and compare them to the new saved data. |

If a refresh matches a dirty draft, the draft becomes pristine. Later refreshes
can then update it normally. A refresh does not clear a previous save error;
the save orchestration or a subsequent edit owns that decision.

The pristine-refresh transition corrects an earlier behavior where entering
edit without changing anything could become dirty after a background refresh,
while still displaying the old configuration.

The hook reconciles changed inputs before React commits children. This prevents
a dashboard switch from briefly committing the previous dashboard's draft.
Equivalent configurations recreated as new objects do not trigger repeated
state updates. Controlled mode changes remain requests to the parent:
`enterEdit` and `exitEdit` do not override the supplied mode prop.

Callers update configurations immutably. The session does not mutate saved
data, but it does not deep-clone every configuration or protect callers that
mutate returned objects directly. API save requests, cache invalidation, and
authorization remain responsibilities of their existing layers.

## Layout and visibility

`dashboardModel` applies grid patches with copy-on-write behavior. The last
patch for a widget ID wins, saved widget order is preserved, and omitted
widgets remain in the draft. Layout sanitization retains registry constraints,
including fixed dimensions. A no-op preserves the original widget array;
changed layouts replace only affected widgets.

View projection filters widgets without modifying the stored configuration.
In the unplugged scenario, the home and DC charging mix widgets each occupy
half the grid. Their wider view layouts never enter the saved draft.

Visibility rules use AND semantics. A nonempty typed rule list takes precedence
over legacy `chargingConnectionVisibility` options. API normalization migrates
legacy options into typed rules while preserving unrelated options. Scenario
previews remain transient renderer state, outside the dashboard schema.

## Verification

The draft contract suite exercises the real hook with fixed synthetic IDs,
frozen input configurations, controlled and internal mode changes, pending
loads, background refreshes, and Strict Mode identity transitions. Model and
visibility tests cover ownership, layout constraints, structural sharing,
legacy migration, and YAML round-tripping. Save-routing and browser editor
tests continue to exercise the unchanged callers.
