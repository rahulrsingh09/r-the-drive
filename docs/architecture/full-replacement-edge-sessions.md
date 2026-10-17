# Edge authentication, sessions, and audit implementation replacement

Documentation impact: internal architecture update. This change preserves the R:The Drive
interface, existing wire/schema contracts, and GPL-3.0-only notices. It is a replacement of
application implementation, not a claim of independent or clean-room authorship.

## Scope and inventory

The baseline is `5651fdfeed91924af623cf7eb919c417e33628bb`. The scope contains 35 Rust
routines across authentication middleware, rate-limit identity selection, session service,
and audit service, plus 36 JavaScript functions, including inline callbacks and worker
`fetch` methods. `middleware/mod.rs` contains two declarations and no executable routine.
The candidate contains 37 Rust routines and 33 JavaScript functions. Detailed per-function
source, hashes, replacement mappings, original assertion hashes, and command logs accompany
the local review evidence under `security/edge_sessions`.

## Application authentication and rate limits

Authentication and rate limiting now share exact Bearer-header extraction and the RS256
claim verifier. Authorization accepts only the case-sensitive `Bearer ` scheme; it does
not promote an access or refresh cookie to a Bearer credential. `panda_` tokens select
integration-key verification before the JWT-extension lookup. Other tokens require the
router's decoding-key extension, the `panda.app` issuer, zero expiry leeway, and session
authorization. The token lifetime remains 900 seconds.

Integration-key selection uses an enabled-user membership query. The route gate uses
explicit exact/prefix tables and an allocation-free vehicle-path template matcher.
The pre-existing prefixes and wildcard segments are preserved, including their edge
cases; this change grants no new routes. Read keys retain one vehicle scope. Only an
allowed request reaches the conditional last-used update, preserving both the one-minute
write interval and database-error precedence on denied requests.

The limiter classifies the credential once as integration key, verified JWT user, or peer.
API-key buckets contain SHA-256 hex rather than the credential. JWT buckets use the subject,
including separate tokens belonging to that subject. Bucket selection does not itself
authorize a session. The trusted-peer gate accepts forwarded addresses only from loopback,
tries `x-real-ip` before a single-address `x-forwarded-for`, and rejects appended chains.
Absent socket information retains the historical loopback fallback.

## Sessions and audit transactions

Session authorization preserves the order: expiry, account existence, account enablement,
then family state. Legacy tokens without `sid` stop after the account gate. A limited join
finds a nonrevoked family belonging to the user with an unrevoked, unexpired refresh token.
Later-table failures cannot replace an earlier authentication rejection.

User revocation locks all matching families in UUID order, then batches family and refresh
updates. Family revocation uses the same batch operation for one ID. The family update and
refresh update remain separate statements: at read committed isolation, a fresh statement
snapshot must see descendants committed while revocation waited for a rotation's family
lock. Combining those updates into one CTE could miss that descendant. Existing revocation
timestamps remain intact, and caller transactions still own commit or rollback.

Audit inserts derive legacy `detail` inside PostgreSQL from the same bound outcome and
target as the structured fields. Metadata remains caller-supplied JSON; this service is
not an arbitrary-secret sanitizer. Callers must continue supplying stable identifiers
and enum-like values. Header request IDs accept UUIDs only. Audit failures propagate and
roll back caller transactions. Retention still deletes only security events older than
365 days, runs immediately, skips missed daily ticks, and stops when its task is aborted.
The explicit deadline calculation is checked against Tokio's original interval behavior.

## Gateway and bounded transfers

The gateway separates configuration, verified admission, route selection, forwarding,
map-description transformation, and private-response construction. Access still requires
an explicit email allowlist, configured public origin, issuer and audience, and RS256
verification. Seven-day session limits, JWKS caching parameters, and origin grammar stay
unchanged. Only the configured HTTPS upstream receives application traffic. Forwarded
edge/Access headers are removed or replaced, Access cookies are removed, and application
Bearer tokens, refresh cookies, browser Origin, CSP, CORS, and Set-Cookie policies survive.
The application remains responsible for its CSRF checks. WebSocket upgrades retain their
original upgrade object. Redirects remain manual and restricted to the configured origin.

Map routing recognizes complete resource grammars, validates tile/glyph bounds, and
reconstructs JSON sources while checking every supported resource URL. The destination
remains the fixed provider; no user headers, credentials, cookies, or query secrets are
forwarded. MIME, PNG signature, JSON schema, timeouts, cache policy, and restrictive map
CSP remain enforced. Configuration overrides still require successful application replies.

Map and backup bodies use a shared bounded collector with geometric buffer growth,
cancellation on incomplete reads, and unconditional reader release. This fixes a measured
baseline defect: successful map reads left the upstream body locked. Backup uploads retain
the exact token grammar, fixed-length digest comparison, age header and checksum checks,
20 MiB cap, write-only endpoint, and 31 daily slots. Receipts follow successful persistence;
failures neither delete the previous archive nor disclose storage diagnostics.

## Fixed-contract operations retained for explicit review

These 12 Rust routine bodies remain token-identical; they are not counted as independently
rewritten algorithms: `JwtKeys::fmt` (secret-free Debug), `issue_access_token` (legacy `sid=None`
forwarding), `AuthIdentityKeyExtractor::new` (key storage), `RateLimitClass::as_header_value`
(five wire strings), `AuditOutcome::as_str` (two stored strings), `SecurityAuditEvent::success`
(the public default record), `target`, `metadata`, `request_id` (field-setting builders),
`record`, `record_tx` (executor/lifetime adapters), and `prune_expired` (one parameterized
retention DELETE and affected-row count). The audit adapters now invoke the new insert.
The JavaScript default `() => Date.now()` and timeout `() => controller.abort()` callbacks
also remain identical. RSA PEM parsing, RS256 encode/decode, SHA-256, the fixed-length XOR
comparison, SQL parameter binding, immutable response fields, and public module declarations
remain constrained by their library or compatibility contracts rather than renamed for
appearance. Parent review should explicitly accept or revisit these cases.

## Validation boundary

The unchanged 26 gateway tests and 45 auth integration tests are retained. Added tests
exercise local HTTP forwarding and negative requests, database expiry/revocation/errors,
transaction rollback, concurrent refresh/revocation and rate limiting, audit persistence,
map rejection/cancellation, key-cache behavior, timeout aborts, and reader release.
Validation uses synthetic records, local key pairs and storage fakes, isolated internal
PostgreSQL/Redis containers, no published ports, pinned Node 24 and Rust 1.97.1, and locked
offline dependencies. Cloudflare deployment behavior, live providers, and external archive
storage are not exercised. Parent integration must rerun its combined acceptance suite.
