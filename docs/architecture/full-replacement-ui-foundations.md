# UI foundation implementation boundaries

The shared UI package separates interaction state and browser resources from the existing rendered markup. Public component signatures, CSS classes, column definitions, formatting defaults, and accessibility attributes remain compatibility contracts.

## State and browser resources

`useColorEditor`, `useDateRangeEditor`, and `useDataTableState` own related state in reducers. A color selection updates the draft and its validation state together. A date edit distinguishes typed input, time adjustments, calendar selection, and external timeframe replacement. Table sorting, column visibility, and context-menu position share a state owner while TanStack continues to provide the row and sorting models. `tableView` projects visible headers, sortable menu choices, row selection, and cell classes for desktop and mobile. Loading rows do not invoke the caller's selection predicate. Optional trip tag and detail columns are selected independently from the ordered column definitions.

`useOverlayInteraction` owns dismissal, focus boundaries, and viewport subscriptions. Select menus retain capture-phase Escape handling so closing an inner picker does not also dismiss its dialog. Dialogs remember the trigger before portal commit, lock body scrolling, cycle focus at the first and last controls, and restore the prior scroll setting and trigger on cleanup. Sidebar navigation retains its separate deferred-focus policy and skips restoring the trigger after route selection.

`overlayLayout` calculates menu placement independently of rendering. Select menus use visual viewport offsets and available space above and below the trigger. Tooltips and theme menus use their existing eight-pixel viewport padding and flip rules. The mobile theme chooser and fullscreen dialog classes remain unchanged. `controlEvents` shares numeric-input conversion, backdrop targeting, focus departure, and ordered selection/close/focus handling. A caller exception during selection still prevents dismissal; no action is caught or retried.

`browserRuntime` groups listener acquisition and cleanup, supports modern and legacy media-query subscriptions, and provides a storage boundary for unavailable browser storage. Unit preference migration retains its three storage keys, write order, and notification events.

## Presentation calculations

Unit formatting uses measurement profiles for precision, labels, scaling, and division. Missing measurements do not read or migrate preferences. Display formatters retain their existing distinction between missing values and nonfinite conversion inputs.

Date formatting shares cached `Intl.DateTimeFormat` instances and one calendar-part projection. Wall-time conversion retains two offset projections, including its existing choice during daylight-saving gaps and repeated hours. Date-only strings keep their UTC calendar interpretation.

Color conversion uses explicit RGB/LMS/Lab matrices and shared channel conversion. Saved colors retain their hexadecimal encoding, rounding precision, and eighteen-step chroma gamut search. A direction-specific transfer specification selects the sRGB linear or power branch for all channels. Coefficients, evaluation order, scalar clamping, and decimal rounding remain numerical contracts. Efficiency conversion shares a finite-value gate while retaining distinct sign rules for reciprocals and energy per distance. Place lookup computes the trip latitude weight once and scans for the nearest inclusive-radius match; equal-distance matches retain the first place.

Theme application materializes CSS variables, chart aliases, series colors, color pairs, and brand assets in one projection. A runtime snapshot is published after its document properties are applied. Authentication reset still selects the safe built-in default without reading theme preferences from browser storage. The bundled chart renderer is unchanged.

Drive-mode aliases use an own-key family registry. Unknown names such as `constructor` and `__proto__` follow the existing fallback text rule instead of raising the inherited prototype-lookup exception; dedicated baseline/candidate regression evidence records this edge-case correction.

Vehicle status uses explicit Boolean and level reading rules followed by one availability/result assembly. Tire presentation shares health-band definitions. Charging cells share the precedence between current live measurements and completed historical measurements. Phantom-drain metric cells share missing-value, precision, and class rules.

## Client diagnostics

Diagnostics are local console records. Cancellation remains silent, repeated error objects are suppressed, and equivalent records expire after five seconds. Global browser handlers prevent raw default error output and return listener cleanup.

Authentication routes, vehicle connection failures, authentication operations, and HTTP 401/403 diagnostics omit raw exception text, stacks, and optional identifiers. They retain a generic error description and an uppercase diagnostic code when available. Other diagnostics remove credential assignments and sensitive headers from text, sanitize caller context labels, and remove URL origins, query strings, fragments, identifiers, and tile coordinates according to the existing path rules. This diagnostic filtering does not alter request handling or authentication decisions.

## Verification and retained contracts

`fullReplacementUiFoundationsContracts.test.tsx` captures baseline outputs for color models, formatting, dates, vehicle presentation, theme materialization, primitive markup, and table markup. Its snapshots were recorded before implementation edits and are retained unchanged. `fullReplacementUiFoundationsInteractions.test.tsx` covers mobile placement, keyboard navigation, dialog focus, sidebar cleanup, chart choice, tooltip descriptions, table sorting/visibility, and editor transitions. `fullReplacementUiFoundationsDiagnostics.test.ts` records the credential-disclosure regression and cleanup/deduplication assertions. `fullReplacementUiFoundationsEdges.test.ts` covers prototype-safe unknown drive labels, missing-measurement storage behavior, and incomplete availability metadata. Existing tests and goldens remain unchanged.

`fullReplacementUiFoundationsRecovery.test.tsx` covers loading/selected/hidden table rows, optional trip columns, row-action isolation, table controls, input feedback, tab thresholds, delta precedence, chart favorites, canceled switches, controlled collapse, numeric boundaries, and place-radius ties. The additional browser fixture compares selected, sorted, hidden-column, and picker states at desktop and mobile widths.

The remaining fixed markup, schema projections, direct callback bindings, public compatibility adapters, and exact scalar operations are individually listed with their body hashes in the scope review inventory. Conditional JSX is reviewed as executable code; it is not a static-template exemption. Retaining those contracts is explicit; it is not a claim that their source text was independently rewritten. No licensing or attribution change accompanies this work.
