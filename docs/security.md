# Security Architecture

## Deployment posture

Panda is not approved for direct Internet exposure. Production Compose binds
the application behind a host firewall and authenticated gateway, and every shared deployment must place
an authenticated tunnel or identity-aware reverse proxy in front of it. The
outer gateway owns public TLS, certificate renewal, identity enforcement, and
Internet-facing rate limits; Panda login remains required behind it.

See the [secure deployment runbook](./runbooks/secure-deployment.md) for the
required gateway contract and verification steps.

The current internal audit evidence and release requirements are tracked in
[`docs/security-audit.md`](./security-audit.md).

Sensitive vulnerability reports must use the repository's [private GitHub
Security Advisory flow](https://github.com/your-org/r-the-drive/security/advisories/new).
Do not publish credentials, exploit details, production data, live telemetry, or
precise vehicle locations in public issues.

## Authentication

- JWT (RS256) with 15-minute access tokens
- Every authenticated request requires an existing, enabled user. Deleting or
  disabling an account denies its access tokens immediately on subsequent requests.
- 30-day HttpOnly refresh tokens, rotated on use
- Disabling an account revokes its refresh tokens and API keys in the same database
  transaction. Deletion removes memberships, vehicle preferences, refresh tokens,
  API keys and OIDC identities, and revokes pending invitations for that account.
  Vehicle records, credential bundles, telemetry and trip history remain intact for
  surviving members. A failed deletion rolls back all database cleanup; existing
  foreign-key restrictions on shared trip tags or referenced cost profiles can
  still require reassignment before deletion. Disable the account to deny access
  while resolving those references.
- API keys are SHA256-hashed, read-only, and bound to exactly one vehicle; keys
  never authorize dashboard, account, administrative, or vehicle-setting writes
- Argon2 password hashing
- Vehicle membership roles are capability boundaries: `viewer` is telemetry and
  history read-only, `manager` may run operational changes such as schedules
  and backfills. Owners manage membership; owners and managers may refresh
  credentials after proving the Rivian account includes the vehicle.
- Enrollment verifies the selected vehicle against the caller's staged Rivian
  account before any membership, credential or vehicle mutation. An upstream
  lookup failure or a vehicle absent from that account fails closed. A vehicle
  already stored locally requires an existing owner or manager membership;
  enrollment never grants access to another local owner's vehicle. Shared access
  must use the membership/invitation flow.
- Live WebSockets require a valid JWT and an enabled vehicle member at the handshake.
  They recheck membership and account status every five seconds and immediately
  before forwarding each telemetry frame, and close when the JWT expires. Failed
  or timed-out database authorization checks close the connection without forwarding
  the pending frame. Checks and socket writes have two-second timeouts. Data already
  sent before revocation cannot be recalled. Clients must refresh their access token
  and reconnect after expiry. There is no SSE telemetry endpoint.
- Protected-route bootstrap uses `POST /v1/auth/bootstrap`, which returns fresh tokens when a valid refresh cookie exists and `204 No Content` when no resumable session exists, so first-load logged-out state does not depend on a visible refresh 401.
- The web app attempts one refresh on protected 401s, then emits a single auth-expired flow: toast, session clear, redirect to `/login`, and resume to the original in-app route after successful sign-in.
- Optional OIDC SSO is configured by a super-user under **Settings > Authentication**.
  It uses the provider issuer/subject identity rather than email alone; local
  password login remains enabled by default and can be restored with the
  documented environment break-glass override.
- OIDC client secrets are write-only and encrypted when stored in the database.
  Authentication settings data is excluded from recovery packages; identity
  mappings remain, so operators must re-enter and test the provider after restore.
- OIDC does not replace the authenticated HTTPS gateway, add provider-driven
  role mapping, or make a directly exposed origin safe.

## Transport Security

- Production nginx is an HTTP origin on port 8080, published on host loopback by
  default through `PANDA_HOST_BIND_ADDRESS`. The separate internal API listener
  needs `ALLOW_PUBLIC_ORIGIN_BIND=true` for a non-loopback bind. Neither is a public TLS endpoint.
- Public HTTPS and HSTS are enforced by the authenticated outer gateway
- `Secure` cookie flag enforced; `COOKIE_INSECURE` is local-development-only.
  The narrow `ALLOW_INSECURE_LAN_HTTP_AUTH=true` production exception accepts
  only documented private literal-IP HTTP origins and emits a startup warning.

## Session and enrollment boundaries

New JWTs identify a refresh-token family. Rotation is serialized on that family;
replay of a consumed token revokes its descendants and immediately invalidates
the family's access tokens. Existing pre-upgrade JWTs without a family retain
their original short lifetime. Missing or disabled users fail authorization.
Administrative disablement also revokes all existing session families, so
reenabling the account requires a fresh login. New session issuance locks the
enabled account while creating the family to serialize it with disablement.
Browser renewals share a cross-tab lock without storing access or refresh tokens.

Enrollment rechecks the selected vehicle against the signed-in Rivian account
before writing local data. First enrollment creates the owner; a verified user
joining an existing vehicle receives Viewer access. Existing roles, telemetry
credentials and collectors remain in place. Owners explicitly promote members.

Live sockets are admitted under per-account, vehicle and process quotas. They
close at JWT expiration and recheck user, family and membership authorization
every 30 seconds. Revoked membership closes with 4403; session expiration or
revocation uses 4401. Messages are limited to 4 KiB and outbound sends to five
seconds. The browser clears removed-vehicle state and renews expired sessions.

## Resource bounds

Full-density metrics stream every retained source point through 10,000-row
database cursor chunks within a repeatable-read snapshot. The JSON contract
and compact-mode sample selection remain unchanged. Admission, deadlines,
bounded output buffers and disconnect cancellation constrain resource use.
Metric and Grafana reads have database statement deadlines. Disconnects and
request timeouts cancel the owned backend query; admission remains held until
its transaction has drained and rolled back, including cancellation cleanup.
Interrupted JSON bodies are failures and cannot become successful query data.
Grafana retains its existing point limit and accepts at most 32 targets.

Basemap data has a shared 128 MiB cache budget and seven-day TTL. A bounded
maintenance scan invalidates legacy permanent entries. Redis uses 192 MiB
`noeviction` beneath its 256 MiB container limit; only basemap entries enter the
application eviction index. An unavailable/full cache produces a cache miss.

## Outbound destinations

Weather, geocoder, OIDC and S3 connections validate current resolved addresses,
pin approved sockets, preserve TLS hostnames, and disable environment proxies
and redirect following. Mixed public/private DNS results and forbidden ranges
fail closed. OIDC and S3 private exceptions are operator-owned environment
CIDRs; custom S3 HTTP additionally requires the explicit trusted-LAN exception.
Actual response bytes are capped for weather, geocoder, Rivian and OIDC HTTP
responses, including chunked bodies.

## Trusted recovery packages

Recovery packages are trusted operator input. PostgreSQL archives execute SQL;
a candidate on the same server is not an SQL sandbox. The restore engine uses
a temporary non-superuser login with no database/role creation, replication,
bypass-RLS, server-file or program privileges. Bootstrap credentials perform
only candidate creation, known extension setup, Timescale pre/post hooks,
baseline comparison, swap and cleanup. Restored objects stay owned by the
unprivileged role, which becomes NOLOGIN with no password after preparation.
An expired restore deadline explicitly terminates the job role's sessions,
disables its login/password, and removes its candidate and orphaned role.
Archive functions and background-job definitions still require trust in the
backup producer; arbitrary SQL risk is mitigated, not eliminated.

One compiled policy controls dump redaction, restore TOC filtering and candidate
sanitation. Provider credentials, cryptographic keys, sessions, API keys,
invitation proofs and operational target rows cannot be imported as active data.
Authentication resets to OIDC off/password login on. Validated non-secret backup
settings and inert history are restored separately; target history wins.
Catalog paths never grant filesystem authority: only ordinary package files
under the configured root are opened/deleted through directory handles. Remote
materialization counts bytes, checks reserved free space, enforces a deadline
and removes partial files. Metadata limits are independent of dump size.
Once admitted, bounded upload/preflight/start work owns its lifetime through
cleanup even if the HTTP client disconnects. Published preflight staging is
removed automatically; only a durable restore handoff retains its package.

## Rate Limiting

- Panda applies class-specific auth, read, write, and heavy-read limits
- The authenticated outer gateway must apply its own client-facing limits
- The internal origin does not trust arbitrary forwarded client-IP headers

## Headers

- `X-Content-Type-Options: nosniff`
- `X-Frame-Options: DENY`
- `Referrer-Policy: no-referrer`
- The app shell uses a restrictive `Content-Security-Policy`; its `connect-src`
  permits same-origin services, WebSockets, and `https://api.github.com` for the
  optional browser-initiated stable release check.

## Database

- PostgreSQL accessible only on internal Docker network (not exposed to host)
- Parameterized queries via sqlx (compile-time checked)
- Telemetry column names validated against allowlist before interpolation
- Rivian vehicle credentials are encrypted with age before durable storage

## Secret Storage

- Durable Rivian credential bundles are encrypted before storage in `panda.vehicle_credentials`
- Short-lived connect / OTP staging data should stay encrypted at rest in Redis and Redis should remain internal-only
- Production requires externally provisioned `AGE_ENCRYPTION_KEY`, `JWT_SECRET`,
  and `JWT_PUBLIC_KEY` (direct values or mutually exclusive `_FILE` sources).
  Startup validates the AGE identity and RSA pair and refuses missing/partial keys.
  Database key storage is limited to explicit local development.
- A public AGE recipient in `system_config` detects accidental replacement. Existing
  DB private key rows require the explicit, transactional [custody migration](./runbooks/key-custody.md).
  It authenticates stored ciphertexts and preserves their bytes before removing only
  the matched key rows. A failed migration leaves the originals intact.
- External keys need independent protected backups. Logical row removal does not
  erase older raw dumps, snapshots, PostgreSQL pages, or WAL containing legacy keys.
  Runtime/host compromise can still expose in-memory keys; separate custody reduces
  database-only compromise risk and does not make a perfect-security claim.

## Audit Logging

- Security events (first-owner claim, login success/failure, password changes,
  account-invitation operations, API-key create/revoke/rotate, and user
  administration) are recorded in `panda.security_events`.
- Each current event has an event type, actor when known, stable target,
  UUID request correlation when supplied, success/failure outcome, and
  redacted enum-like metadata. The service must not store credentials, tokens,
  email addresses, locations, or raw telemetry in this audit metadata.
- The application retention worker purges live security events older than
  365 days. Recovery packages may retain older events under the operator's
  separate backup-retention policy.
- Owner-started ingestion captures are stored in
  `panda.vehicle_ingestion_capture_events`. They hold sanitized ingestion
  facts, decoded values, and raw Parallax payload bytes for topics that cannot
  carry location or network identity. A key filter removes vehicle IDs, VINs,
  names, coordinates, and credential-like fields before storage and again on
  export. Each vehicle keeps one capture, which stops after one hour and is
  purged 24 hours after it stops.
- Structured `[panda][LEVEL]` key-value logs are written to stdout/stderr; Docker supplies the outer timestamp. The production wrapper normalizes Nginx error lines into the same shape.

## Security regression controls

- Backend authorization regressions use synthetic Rivian responses, real local
  PostgreSQL/Redis and actual WebSocket clients. From `apps/api`, run
  `cargo test --locked --lib authorization_ -- --ignored --test-threads=1`
  with disposable TimescaleDB `DATABASE_URL` (including `CREATEDB` permission) and
  `REDIS_URL`. Each test creates and removes its own migrated database; no real
  Rivian credentials or vehicle connection are needed.
- `pnpm security:routes` verifies that every API route module is mounted through
  the intended protected router composition. The authentication module must
  have separate public, metadata, and protected mounts.
- Restore capability tokens and agent keys are compared without early-exit
  string equality and reject oversized or malformed authentication headers.
- Dynamic telemetry and metric selectors reject control characters and bounded
  oversized input before their allowlists are used to build SQL identifiers.

## Dependencies

- Weekly automated dependency audits via Dependabot
- `cargo audit --deny warnings` in CI
- `pnpm audit --json` in CI (production and development dependencies; raw findings retained and narrowly reviewed exceptions checked by `scripts/check-dependency-audits.mjs`)
- Semgrep SAST is blocking on trusted branches and same-repository pull
  requests; fork pull requests use a separate secret-free blocking scan.
- Fixable critical and high Trivy findings are blocking after the unified production image builds; unfixed base-image findings remain visible for review and base-digest refreshes.
- Workflow actions are pinned to reviewed commit SHAs.

## Release Images

- Standard Compose pulls one public unified image from GitHub Container Registry; source builds use the explicit build overlay only.
- Stable images use immutable Calendar Version tags and provenance attestations; `latest` is a moving convenience tag, not a reproducible deployment identifier.
- Versioned container images are published only by intentional release workflows from validated `main` tags or the current `dev` pre-release candidate. The manually dispatched candidate workflow may also move the mutable `:dev` alias after it verifies the exact current upstream `dev` SHA; this alias is for development and test environments, not production release identity. Manual Candidate image dispatches may publish unversioned, commit-addressed build candidates; these are cache/release inputs, not releases. Stable and pre-release image tags and digests must be treated as release artifacts.
- See the [release images runbook](./runbooks/release-images.md) for package visibility, tag protection, and recovery requirements.

## Production Checklist

- [ ] `COOKIE_INSECURE` is NOT set (except local development)
- [ ] `ALLOW_INSECURE_LAN_HTTP_AUTH` remains `false`, or the documented trusted-LAN exception and host firewall controls are in place
- [ ] `POSTGRES_PASSWORD` changed from default
- [ ] `REDIS_PASSWORD` is strong and Redis is not host-published
- [ ] Production AGE identity and RSA signing keys are stored safely outside the database and backed up separately
- [ ] `ALLOWED_ORIGINS` set to exact frontend domain(s)
- [ ] An authenticated tunnel or identity-aware reverse proxy terminates public HTTPS
- [ ] Host firewall rules restrict direct access to port 8080
- [ ] Redis is reachable only on a private/internal network
- [ ] Firewall blocks API, PostgreSQL, Redis, and origin ports from external access
- [ ] `PANDA_IMAGE` explicitly selects the reviewed digest built from the tested source

### Rivian error logging

The telemetry fork withholds raw Rivian GraphQL error messages, codes and reasons
from returned error strings and logs. Those fields can reflect request inputs.
Authentication categories are classified internally before details are discarded.
Production container builds enforce the committed Cargo lockfile.

Request tracing uses matched route templates, including for invitation URLs,
without query strings or request headers. The proxy records coarse API prefixes
and response status only. Raw proxy error logging is disabled because Nginx embeds
request URLs in those messages; application error logs and proxy status records
remain available. The same security headers are included on the HTML shell and
static responses, with a same-origin Content Security Policy and local blob
workers for map rendering.

Dashboard startup disables Iconify network loading before widgets render, and
opening the icon picker cannot restore that transport. Uncached remote icons and
online icon searches remain unavailable. The initial theme uses static HTML
attributes, so the page does not need an inline-script exception.

The Cloudflare gateway has an opt-in, fixed OpenFreeMap proxy. This exception
preserves the API's optional-provider denial and the browser's same-origin CSP.
It validates Access before serving public cartography, verifies the normal app
response before adapting map configuration, strips credentials on map downloads,
and enforces fixed paths, redirect denial, type checks, timeouts and size bounds.
Map tiles contain no private telemetry; app authorization remains required for
trip data. Cloudflare-added IP headers may reach OpenFreeMap. See the
[gateway map contract](guides/private-origin-gateway.md#optional-free-street-maps).

### Runtime packages and recovery

The runtime installs current distribution security updates, retains the PostgreSQL
client tools needed for recovery, and removes the unused database server, JIT,
XML/SQLite libraries and GPG tooling. Setuid and setgid executable bits are removed.
Review the finished image before each deployment; a pinned base digest alone does
not include later distribution fixes.

The October 5, 2026 image scan still reports distribution advisories. The remaining
critical entry, [CVE-2023-45853](https://security-tracker.debian.org/tracker/CVE-2023-45853),
concerns MiniZip: Debian documents that the affected code is not built into
Bookworm's zlib binary packages. Remaining high entries include privileged host
utilities, archive/LDAP paths not used by the application, curl modes beyond the
fixed local health probe, and OpenSSL DTLS, which this deployment does not use.
These are applicability observations for this configuration, not blanket
exclusions or a clean vulnerability scan. Reassess them when changing the image,
networking, subprocesses or runtime privileges.

Recovery extraction preserves validated empty directory entries, including an
empty vehicle-image cache. An installation with third-party artwork disabled can
restore a backup without previously downloaded images.
