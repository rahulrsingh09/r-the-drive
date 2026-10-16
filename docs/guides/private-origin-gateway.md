# Protecting the private origin

The fork can require an additional credential between an authenticated reverse proxy and nginx. This prevents direct requests to the origin from bypassing the outer access policy. It does not replace the application's login or vehicle authorization.

Set both variables on the application container:

| Variable | Value |
| --- | --- |
| `PANDA_REQUIRE_GATEWAY` | `true` |
| `PANDA_GATEWAY_TOKEN` | A randomly generated secret containing 43–128 ASCII letters, digits, underscores or hyphens |

Generate at least 32 random bytes and encode them with base64url. Store the value in the hosting provider's runtime secrets and in the trusted proxy's secrets. Never include it in Git, a browser bundle, a URL, or a build argument.

The trusted proxy must authenticate the visitor, replace any incoming `X-Panda-Edge` header with the configured secret, and forward the application's `Authorization` header unchanged. Use HTTPS between the proxy and a remote origin.

The container validates the settings before starting nginx or the API. A required but missing or malformed token stops startup. Valid configuration is written with mode `0600` under `/tmp`. Requests without the exact, case-sensitive header receive HTTP 403, including static assets and recovery routes. nginx removes the gateway header before forwarding requests to the application. The existing `/health` endpoint remains available for platform probes and does not expose account or vehicle data.

Leaving both variables unset preserves deployments where private networking already restricts origin access. Do not expose that configuration directly on the Internet. For a remote origin that relies on this gate, keep `PANDA_REQUIRE_GATEWAY=true` so removing a secret cannot silently disable protection.

After deployment, verify that requests with a missing, incorrect, or case-altered header fail, while a valid header permits the normal application login. Confirm that the public proxy authenticates visitors before adding the header. Rotate the secret in both services if it is disclosed.

## Cloudflare Worker

The optional `apps/private-gateway` Worker authenticates Cloudflare Access JWTs
before forwarding HTTP requests or WebSocket upgrades to the origin. It checks
the signature, expiration, issuer, application audience, and explicit email
allowlist. It does not replace the Panda owner login.

Configure a Cloudflare Access self-hosted application for the Worker's exact
production hostname, with an allow policy limited to your own email. A free
`workers.dev` hostname can be used. Disable preview URLs. Confirm that Workers
and Access are on their free plans before deployment; this repository does not
activate or upgrade a subscription.

### Remembering access for seven days

Cloudflare has a global session on the Access team domain and an application
session on the dashboard's hostname. A longer global session can renew a shorter
application session without another email code, but that requires the browser
to retain and send the global-session cookie.

For seven-day access without an eight-hour renewal, set both the Access
application **Session Duration** and the global session duration to `168h`.
Keep the owner allow policy's duration set to **Same as application**. The
gateway accepts correctly signed application tokens for at most seven days and
rejects tokens whose declared lifetime exceeds seven days. Signature, issuer,
audience, expiry, owner allowlist, and separate Panda authentication checks
remain required.

This extends the lifetime of the application access cookie itself. A copied,
still-valid cookie is usable for longer, so this is a convenience/security
tradeoff, not just a display setting. It grants no additional account permissions
and does not change Rivian credentials or vehicle data. Cloudflare Access session
revocation remains available.

Deploy this gateway version before increasing the application duration; older
versions rejected tokens more than 24 hours after issuance. Existing cookies may
keep their original expiry, so sign in once after the change to obtain a new
session. Changing browser profiles, using private browsing, clearing cookies,
signing out, or revoking a session can require an earlier login. Seven days is
the configured maximum, not an availability or browser-persistence guarantee.

When investigating a prompt, distinguish an Access renewal/Continue page from a
new email-code challenge. Do not diagnose a browser-cookie problem from server
configuration alone or solve it by bypassing Access.

### Deployment configuration

Copy `apps/private-gateway/wrangler.example.json` to a private deployment
configuration and fill in its five required variables. `PUBLIC_ORIGIN` is the address
you will open, `UPSTREAM_ORIGIN` is the Northflank HTTPS origin,
`ACCESS_ISSUER` is your Cloudflare Access team URL, `ACCESS_AUDIENCE` is the
Access application's audience tag, and `ALLOWED_EMAILS` is a comma-separated
list of exact email addresses. Wildcards are not accepted.

Set the Worker's `GATEWAY_TOKEN` secret to the same value as the origin's
`PANDA_GATEWAY_TOKEN`. Keep secrets out of configuration files and Git.
Run `pnpm --filter @panda/private-gateway test` and
`pnpm --filter @panda/private-gateway build` before deployment. The Worker
returns an error when setup is incomplete; keep the origin private until the
Access policy, Worker configuration, and secrets are ready.

Fork CI also runs `apps/private-gateway/runtime-check.mjs` in the Workers
runtime with synthetic data. This verifies signed JWT authentication, forwarded
HTTP bodies and cookies, and a real WebSocket upgrade and message exchange.
To reproduce it, install `miniflare@5.20261001.0-alpha` in a separate temporary
directory and set `PANDA_MINIFLARE_MODULE` to that directory's
`node_modules/miniflare` path. The runtime needs a supported operating system;
on an older Linux host, run the check inside an Ubuntu 24.04 container.

Add the Worker's public origin to the app's `ALLOWED_ORIGINS` runtime setting.
When a remote Worker needs to reach Northflank, its HTTP port must be reachable,
but the verified nginx gateway credential must remain required. Check both
layers: direct origin requests without the credential must fail, and Worker
requests without a valid allowed identity must fail. Recheck browser login,
cookie renewal, trip details, and live WebSocket updates through the public
Worker address before connecting a real Rivian account.

Access credentials, including the CLI `Cf-Access-Token` header, are stripped
before proxying. App authorization, refresh
cookies, request bodies, and WebSocket protocols are preserved. The proxy uses
a fixed origin, never follows origin redirects, rejects alternate hostnames,
and disables application-response caching. Worker request logging is disabled in the
example configuration. Cloudflare processes the proxied traffic, including
login requests; it is part of the hosting trust boundary.

Workers Free currently allows 100,000 requests per day and 10 milliseconds of
CPU per request. Confirm current allowances and existing account usage in
Cloudflare before enabling the proxy. Reaching a free quota can interrupt
access; it is not a reason to upgrade automatically. Telemetry collection
runs on Northflank and does not depend on an open browser or this proxy.

## Optional free street maps

Set `ENABLE_FREE_MAPS=true` on the Worker to enable only OpenFreeMap. The default
is `false`. This is a gateway capability; it does not relax the API's optional
outbound policy, enable vehicle commands, or unlock custom provider URLs.
No database migration or additional service is needed.

The existing authenticated basemap configuration response is adapted only after
the API returns HTTP 200. App login failures and disabled-user responses remain
unchanged. Public map assets require the same verified Cloudflare Access identity
as the rest of the gateway, but do not require an app session: those assets contain
public cartography, never trip geometry or account records. Private vehicle routes
still require the application's normal authorization.

The Worker downloads maps directly from the fixed HTTPS host
`tiles.openfreemap.org`, avoiding Northflank map-download bandwidth and disk use.
It accepts only known style, tile, sprite and glyph paths, bounds zoom/coordinates,
blocks redirects, discards request credentials/cookies and provider cookies, and
limits responses to 4 MiB (512 KiB for styles) with a 10-second timeout. Resource
URLs in styles and tile manifests are validated and rewritten to the existing
same-origin map paths. The browser's external-network CSP stays unchanged.
Browsers may cache public cartography for five minutes (style/manifest) or one day
(other assets); application data remains `no-store`.

OpenFreeMap receives requested map areas and network metadata. Cloudflare may
forward the viewer's IP in platform-added headers; this is not anonymous map
access. No Rivian credentials, application tokens, trip IDs, or route polylines
are deliberately sent to the map provider. Styles include OpenFreeMap,
OpenMapTiles and OpenStreetMap attribution.

OpenFreeMap's public instance currently requires no account, API key, or payment.
Map downloads still consume Workers requests, sharing the Free plan's daily
allowance with the dashboard. No paid resource, storage binding, subscription
upgrade, or automatic upgrade is configured by this feature. Hitting a Free
limit can interrupt maps and dashboard access; the Northflank collector continues
independently. Free hosting does not guarantee availability or future pricing.
Review [OpenFreeMap](https://openfreemap.org/) and
[Workers pricing](https://developers.cloudflare.com/workers/platform/pricing/)
before deployment.

Settings reports the gateway map provider as enabled and read-only. Its API-side
request/cache counters do not measure gateway traffic; consult Cloudflare usage.
Other optional providers stay disabled. To disable street maps, deploy
`ENABLE_FREE_MAPS=false`; map configuration is not cached and existing public map
assets in browser caches contain no account data. The API-only installation
continues to show routes on a neutral background.
