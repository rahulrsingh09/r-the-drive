# Privacy

Panda is designed so the dashboard, database, and backups live in infrastructure you operate. It does not include product analytics, tracking pixels, or telemetry sent to a Panda-operated analytics service.

## What stays with your installation

Vehicle telemetry, account records, dashboards, and application data are stored in your Panda database and any backup destination you configure. Rivian credentials are encrypted at rest using the installation's age key.

Your host, reverse proxy, identity provider, and backup provider have their own logs and retention policies. Configure them to match your privacy expectations.

Vehicle ingestion diagnostics are off by default and can be enabled by an owner or manager for one hour at a time. They record field coverage, timestamps, and decode outcomes for troubleshooting; they do not record raw upstream payloads, credentials, or precise coordinates.

## Requests to other services

Panda still needs to communicate with services that make its features work:

- **Rivian:** account authentication, read-only vehicle telemetry, and charging history.
  The fixed HTTP/WSS destinations reject redirects. Charging and departure/preconditioning
  mutations are denied by the backend. Tokens themselves are **not provider-enforced
  read-only**; a compromised server or stolen token retains its provider permissions.
- **Optional location/media providers:** Open-Meteo, Nominatim, basemaps, Iconify,
  and new vehicle-artwork downloads are disabled at the API in this fork, even if old saved
  settings enable them. Custom/self-hosted provider paths and synthetic provider
  tests are disabled too. Existing local data and artwork are retained. Browser
  artwork sources must be local, and Google Fonts is no longer requested.
- **Optional gateway maps:** `ENABLE_FREE_MAPS=true` enables fixed OpenFreeMap
  cartography through the Cloudflare gateway. OpenFreeMap receives map areas and
  network metadata, potentially including viewer IP headers added by Cloudflare.
  The proxy sends no application/Rivian credentials, cookies, trip IDs, or private
  route geometry to OpenFreeMap. Public map tiles, label glyphs, and map symbols
  may be cached in the browser. Private account and trip responses remain uncached.
- **Your configured S3-compatible backup service:** backup uploads, only when you enable it.
- **Your configured OIDC provider:** when SSO is enabled, Panda sends the
  server-side authorization-code exchange and receives the claims required for
  login or explicit account linking. The provider's issuer, subject, email, and
  claim data are governed by that provider's privacy policy and logs. Panda
  does not send vehicle telemetry or Rivian credentials to the OIDC provider.

**Settings > External Connections** reports the effective disabled state of
optional API providers. Administrators cannot override this deployment guard with
saved settings. The gateway reports its optional fixed map provider separately
as enabled and read-only. Without it, maps retain trip geometry on a neutral background; missing
vehicle images use packaged fallbacks. See [external connections](guides/external-connections.md)
for the disabled custom-provider paths and the requirements for any future opt-in.

OIDC client secrets are write-only in the UI and are excluded from recovery
packages. Review the provider's claim and retention settings before enabling
automatic signup or verified-email account linking.

## Bundled demo data

Administrators can create R1T, R1S, and R2 demo vehicles without Rivian credentials. Their rolling 14-day history is illustrative and read-only: it uses aggregate density measured from a human-reviewed development sample, generated sensor values, and deterministic routes between public landmarks around Washington, DC. It does not contain source VINs, account identifiers, payloads, addresses, timestamps, coordinates, or route geometry.

Demo trips include stored route previews, reserved fixture addresses, and completed weather records. Viewing or refreshing a demo therefore does not call Rivian, Nominatim, or Open-Meteo. Refresh replaces only that demo vehicle's generated history, runtime status, and fixture artwork records; memberships, sharing, display names, preferences, and dashboard customizations remain intact.

## A practical reminder

Self-hosting gives you control; it does not make network traffic disappear. Keep Panda behind authenticated access, secure your host and backups, and avoid sharing screenshots or logs that contain account details, locations, tokens, or vehicle data.
