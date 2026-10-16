---
title: External connections
description: Configure weather, geocoding, basemap, proxy, privacy, and self-hosted providers.
slug: /using-panda/external-connections/
---

# External connections

Open **Settings > External Connections** to inspect provider status. This fork's
telemetry-only policy disables Open-Meteo, Nominatim, basemaps, Iconify, and new
vehicle-artwork downloads. The application also uses local/system font fallbacks
instead of contacting Google Fonts. Existing trip data, cached labels, weather
history, and local artwork are retained. Without a gateway map capability, maps
show routes on a neutral background.

The private Cloudflare gateway can enable a fixed OpenFreeMap integration with
`ENABLE_FREE_MAPS=true`. Settings then reports this map provider as enabled and
read-only. This exception serves public cartography through the gateway, leaving
the server's other optional providers and custom URLs blocked. See
[free street maps](./private-origin-gateway.md#optional-free-street-maps) for
privacy, free-plan limits, and disabling the capability.

The guard applies at request time and cannot be relaxed in Settings or by old
saved `enabled`, `custom`, API-key, bearer-token, or private-network allowlist
values. Provider tests and attempts to enable these connections also fail closed.
There is no environment opt-out. OIDC and explicitly configured S3 backups have
separate controls and are not governed by this optional-provider guard.

Custom location/media endpoints are **unavailable**, including self-hosted ones.
This closes the request-time DNS/private-address and unbounded-download exposure
by preventing those requests, rather than claiming all inherited custom-provider
implementations are safe. Re-enabling them requires a code review and request-time
DNS pinning, private-address rejection, redirect denial, and streamed size limits
for every download path. The artwork downloader additionally has a 5 MiB response
limit. The browser rejects external artwork sources and uses packaged fallbacks.

The following sections describe inherited upstream provider contracts for
maintainers; these provider modes and their enable/test controls are unavailable
in this build.

## GitHub Releases

The Settings heading shows the running build version and links to the [Panda
releases page](https://github.com/your-org/r-the-drive/releases). GitHub
Releases is listed alongside the other external connections and controls the
optional checks. The sidebar download icon appears only when a newer release
exists, opens the release page, and does not install or apply an update.

Optional release checks are disabled by default. An administrator or super user
can enable them for the installation and choose hourly, daily, weekly, or
monthly checks (every 30 days). While enabled, each browser checks on its first
open and then at the selected interval while Panda is open. The browser
requests GitHub's [latest release endpoint](https://docs.github.com/en/rest/releases/releases#get-the-latest-release)
directly; GitHub's endpoint selects the latest published stable release and
excludes drafts and prereleases. The request adds no user, vehicle, or usage
data. GitHub receives the ordinary network metadata needed to serve a browser
request, while Panda receives no check counts or results. The last attempt
and release result stay in that browser.

## Weather and outside temperature

Rivian's usable vehicle-state subscription currently provides cabin and driver-set temperatures but rejects its exterior-temperature field. Panda therefore estimates exterior temperature after a drive using Open-Meteo.

- The drive start, end, and each 15-minute point are selected from the exact local route.
- Provider coordinates are rounded to roughly 1 km by default, deduplicated, shuffled, and sent in batches of up to 50 locations.
- Exact route points and timestamps stay in Panda. Returned hourly temperatures are mapped back to the local timeline.
- Trips less than five days old use the forecast endpoint; older trips use the archive endpoint.
- The trip summary is a time-weighted average. The same value powers the trip timeline, average-outside-temperature card, and efficiency temperature buckets.
- Stored samples say whether values came from `vehicle`, `open_meteo`, or both. If Rivian supplies a raw exterior value in the future, it wins at covered times.

Upstream supports remote Open-Meteo; this fork keeps it disabled on install and upgrade. Upstream administrators can choose a custom forecast and archive URL, add a write-only encrypted API key, select exact weather coordinates, or disable weather. Disabling weather pauses queued jobs and preserves history. An Open-Meteo-compatible endpoint is the supported self-hosted contract.

After a fresh install or sanitized restore, this fork creates disabled optional-provider records. Existing records remain stored but cannot bypass the runtime guard. Custom endpoints and encrypted secrets are never recreated. Completed trips with usable coordinates are reconciled in bounded background batches for missing weather and reverse-geocoded locations; trips without usable coordinates are reported as unavailable rather than retried indefinitely.

## Geocoding

Public mode sends exact coordinates or explicitly submitted search text to Nominatim through the Panda server. Exact values are necessary for accurate addresses. Panda checks saved places and cached results first, identifies itself with a static project User-Agent, and limits public requests to one per second. The public service is never queried on every keystroke.

A custom or self-hosted Nominatim base URL can optionally enable debounced autocomplete. The custom selector includes the standard Nominatim-compatible contract and a local Nominatim example; it still uses the normal `/search` and `/reverse` paths. Disabling geocoding stops new searches and automatic trip labels; coordinates, saved places, and cached labels remain.

## Basemaps

Trip geometry remains exact. Basemap resources are requested through an authenticated Panda proxy, so providers see the requested map area and the server connection, not each viewer's browser identity. **Automatic** uses CARTO when an encrypted CARTO Basemap key is stored and OpenFreeMap when it is not. Administrators can pin OpenFreeMap or CARTO; clearing a CARTO key while CARTO is pinned returns the policy to Automatic and therefore OpenFreeMap. CARTO keys are write-only and sent only server-side to CARTO-owned hosts, never to an arbitrary custom tile provider or back to a browser. Request this product-specific key from [CARTO's Basemap-key page](https://carto.com/basemaps/apikey/); it is different from a CARTO API Access Token. Custom XYZ raster templates still require `{z}`, `{x}`, and `{y}`, attribution, and may include an encrypted bearer token. Saving or clearing basemap settings purges the server cache and gives the browser a fresh first-party resource URL.

When OpenFreeMap is resolved, each user can choose Follow appearance, Positron, Bright, Liberty, Dark, Fiord, or 3D (Liberty) in **Settings > Appearance**. Follow appearance selects the light/dark style with the application theme; named styles remain pinned across theme changes. 3D uses Liberty with a pitched, rotatable camera and resets to a flat camera when leaving 3D. OpenFreeMap and CARTO attribution is rendered as accessible links on the map. The browser contacts only Panda's same-origin basemap paths; style JSON, vector tiles, sprites, and glyphs are proxied and revisioned.

Choosing **Disabled** keeps the route on a neutral background. Custom XYZ raster tile servers are supported, including TileServer GL-compatible raster templates; the selector provides a TileServer GL local example. HTTP is accepted only for a confirmed RFC1918 or IPv6-ULA private endpoint with an explicit CIDR allowlist; loopback, link-local, cloud-metadata, unspecified, multicast, and other forbidden ranges are never allowed.

## Local provider caches

Panda keeps a persistent local cache for basemap tiles and address-search results. Reopening a map or repeating an address search uses the local cache instead of contacting the provider again. Reverse-geocoded address records are also stored in the database and survive restarts.

Use **Test with synthetic data** in External Connections to verify a provider without sending vehicle history. If a map has route geometry but its basemap is unavailable, Panda keeps the route visible on a neutral background and identifies the failure. **Map configuration unavailable** means the signed-in basemap configuration request failed; **Map tiles unavailable** means a configured style or tile resource failed. Each state has its own retry action, and actionable browser diagnostics remain available in the local console.

The selected connection shows its entry count and storage use. Administrators can use **Purge cache** when a provider changes data or storage needs to be reclaimed. Purging map tiles means the next view may request a tile again. Purging Nominatim removes lookup-only address records and search results, while preserving addresses attached to trips, charge sessions, or saved places.

## Other connections

- **Iconify:** catalog searches and runtime icon files use the Panda proxy. Disabling it preserves bundled icons and existing supported selections.
- **S3-compatible backups:** status is shown here, while endpoint and credential controls remain in **Settings > Backups**.
- **Rivian account:** vehicle connectivity and artwork remain managed from **Settings > Vehicles**. Rivian-provided artwork is fetched with the same encrypted account session used for telemetry, stored under the persistent application cache directory, and served only from first-party cache URLs. While an image is missing, loading, or being repaired, Panda immediately shows the packaged fallback for that vehicle model; the browser never requests a Rivian image URL. Administrators can use **Refresh vehicle artwork** from the vehicle card to fetch a new manifest and invalidate immutable image URLs.

## Upstream custom endpoint policy (disabled here)

Custom endpoints accept HTTPS. HTTP is restricted to confirmed private
destinations. Private service access requires a non-empty CIDR allowlist made
only of RFC1918 IPv4 or IPv6 ULA ranges; the resolved private address must be
inside that list. A legacy private-network opt-in is deliberately represented as
`migration_required` and is disabled until an administrator saves explicit
allowlists. The connection inventory exposes the policy state and canonical
allowlist but never a stored secret.

The inherited basemap proxy resolves the hostname both while validating a change
and immediately before each outbound request. This does not establish safety for
all inherited weather/geocoding/image paths, which remain disabled here. The proxy rejects mixed public/private results and pins
the request to the approved resolved addresses while retaining the hostname for
HTTP Host and TLS SNI. It does not follow redirects, so a provider cannot turn
an approved URL into a second destination. The service also rejects executable
and file schemes, credentials/query strings/fragments in custom URLs,
link-local and cloud-metadata addresses, bounds proxy responses, and never
returns stored secrets to the browser. Connection logs omit coordinates,
addresses, search text, query strings, credentials, VINs, and vehicle names.

## Upstream connection verification (disabled here)

Use **Test with synthetic data** before relying on a provider. The result is separate from runtime health, so testing an unsaved endpoint never overwrites the installed provider's last-success record. Each result shows named checks and safe messages; it uses a generic location, map tile, or icon name rather than a real drive.

For a release or a new self-hosted endpoint, verify every enabled connection, then inspect a signed-in browser's network panel. Browser requests should target only Panda's same-origin basemap and Iconify proxy paths. A configuration or tile failure switches the map to a neutral recovery state with the matching retry action; it must not leave a blank interactive map. See [Frontend error observability](../architecture/frontend-error-observability.md) for the safe diagnostic fields and expected quiet failures.

## Destination checks and bounded caches

Provider DNS is checked for every new outbound request and approved addresses
are pinned for the connection. Mixed public/private answers, metadata and
link-local destinations, and redirects fail closed. Private weather/geocoder
CIDRs must match the saved connection policy; OIDC/S3 private CIDRs are operator
environment settings. Response bodies are bounded by actual bytes, including
chunked transfers.

Basemap resources share a 128 MiB cache budget and expire after seven days by
default. Least-recently-used basemap entries can expire earlier under pressure;
operational Redis keys do not participate. Legacy permanent tile entries are
invalidated in bounded scans during upgrade, so some map requests become cold
cache misses. Manual purge remains available.
