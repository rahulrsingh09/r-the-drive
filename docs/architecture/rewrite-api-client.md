---
title: API client request and response stages
description: Endpoint contracts, session execution, and compatibility checks for the R API client.
---

# API client request and response stages

The API client keeps the existing R interface and PANDA/panda identity. This
implementation continues the repository's GPL-3.0-only code and NOTICE
attribution. The rewrite was informed by the existing implementation and its
contracts.

## Ownership and flow

`packages/hooks/src/api/clients.ts` owns all 155 named endpoint and session
methods. Each JSON endpoint describes its method, path, body, query parameters,
error reporting preference, and optional response decoder. The seven existing
domain clients bind methods to the same authenticated instance.

`apiClientTransport.ts` owns execution and session state. `transport.ts` retains
the existing class, singleton, URL configuration, and type exports. The
`api.ts` and package facades, consumers, and `themeClient.ts` are unchanged.

Execution proceeds through these stages:

1. Check the existing rate-limit class cooldown.
2. Normalize the request URL, query parameters, and JSON body once. Keep
   `undefined` bodies absent and retain the distinction between omitted and
   empty query parameters.
3. Fetch with `credentials: include`, the current bearer where allowed, and the
   caller's signal and extra headers.
4. For an eligible 401, renew the session and replay at most once. Reuse a token
   another request already renewed when a delayed 401 arrives.
5. Decode HTTP errors independently of successful JSON. A raw response remains
   unread for consumers that need headers, binary data, or their own decoder.
6. Decode successful JSON, then apply the endpoint's compatibility conversion
   from `normalization.ts`.

Serializing once also prevents caller mutation during renewal from changing
the body of a replay. Serialization failures propagate before any network
request.

## Session and failure contracts

Public authentication paths retain the existing bearer exclusions and never
trigger automatic renewal. All other authorization checks remain on the server;
the client adds no permissions or network endpoints. Session tokens remain in
memory.

Automatic renewal, manual refresh, and bootstrap share one pending promise per
instance and call the unchanged `withSessionLock`. The existing cross-tab lock
name and fallback implementation remain intact. Renewal has a 30-second network
timeout; a caller's cancellation does not cancel the shared renewal. An aborted
waiter cannot replay after renewal.

An explicit token replacement or session clear invalidates pending work from the
previous session. Renewing a token within the same session keeps concurrent
requests valid. A stale renewal cannot publish tokens or clear a newer session;
a stale protected request cannot replay or return data under the newer account.
These operations reject with `SESSION_CHANGED`, without an auth-expired event.
Publishing the same non-null token again remains valid for existing auth-store
consumers. A session clear also invalidates a pending tokenless bootstrap.

Connection failures and aborts use `NETWORK_ERROR` with the original cause.
Interrupted or syntactically invalid successful JSON uses `INCOMPLETE_RESPONSE`.
Endpoint conversion failures use `INVALID_RESPONSE`. Malformed HTTP error
envelopes retain their status and request ID with safe code/message defaults.
Malformed renewal responses preserve the current session rather than announcing
an authentication failure. A successful renewal publishes tokens once.
Throttled renewal also preserves the session and returns the original 429 and
retry metadata. The renewal class cooldown prevents another network renewal
until its deadline; rejected renewal responses (401/403) still expire the session.

Rate-limit classes, header precedence, the five-second default cooldown, toast
events, and auth-expired event deduplication are preserved. There is no automatic
rate-limit or network retry. Public methods that suppress routine error reports
continue to do so; session-expiration reporting retains its existing behavior.

## Compatibility boundaries

The conversion layer preserves canonical-field precedence over legacy aliases,
numeric-string conversion, null and zero distinctions, pagination envelopes,
timeframe parameters, trip-tag sorting, location labels, and charging live fields.
`chargingSessionFields.ts` is unchanged. Missing list-envelope fields still
produce empty arrays; invalid non-array list values are rejected.

Trip-list/map tag filters retain their existing treatment of simultaneous tag
IDs and `untagged`. Efficiency filters retain their existing precedence of tag
IDs over `untagged`. The tire-pressure timeline stays available on the main API
facade; the domain method sets are unchanged.

Charge normalization preserves the existing uppercase `AC`, `DC`, and `DCFC`
acceptance. The shared `ChargeSession` type uses a different charger vocabulary;
this pre-existing mismatch remains a dependency for the shared-types owner.

Specialized transports retain separate contracts:

- Backup upload uses XHR for progress, raw file bytes, included cookies, bearer
  authorization, and the sanitized `X-Panda-File-Name` header. It does not renew
  or retry. Aborted uploads settle as interrupted, and malformed successful
  upload envelopes are rejected.
- Restore-job polling uses only the restore capability header and same-origin
  cookies, without bearer injection or renewal.
- First-party map/icon proxies use the existing origin and path allowlist.
- Protected artwork keeps its existing path restriction and normal session
  transport. Download decoders retain bytes and filename fallbacks.

Cross-tab locking, outbound policy, security modules, schemas, persisted
data, endpoint paths, and remote providers are outside this rewrite.

## Verification

The synthetic contract fixtures and snapshots in
`apps/web/src/test/rewriteApiClient.*` were recorded against
`8e32427e7b0c1927f4dd0c55984dd2462e4505e0` before implementation replacement.
They cover requests and successful responses for 154 named methods, with the
remaining XHR method covered by transfer tests. Separate tests exercise
normalization, malformed responses, errors, cancellation, session concurrency,
retry limits, cookie and bearer behavior, and cooldowns. Every request is mocked.

Run the focused suite from the web workspace with `pnpm -C apps/web exec vitest
run src/test/rewriteApiClient.contract.test.ts
src/test/rewriteApiClient.transport.test.ts
src/test/rewriteApiClient.normalization.test.ts
src/test/rewriteApiClient.transfers.test.ts`. Shared-seam validation also includes
workspace type checking, `pnpm build`, `pnpm docs:check`, and `pnpm docs:build`.
