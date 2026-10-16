---
title: External connections
description: Understand provider defaults, optional weather and backups, gateway maps, and blocked connections.
slug: /using-panda/external-connections/
---

# External connections

Open **Settings > External Connections** to inspect effective provider status.
R:The Drive keeps optional data sharing off until it is configured explicitly.
Some provider types are blocked by the runtime policy and cannot be enabled in
Settings, including through previously saved configuration.

| Connection | Fresh-install default | Supported configuration |
| --- | --- | --- |
| Rivian account | No vehicle credentials | Connect your account in Settings > Vehicles for read-only telemetry and charging history. |
| Open-Meteo weather | Disabled | Opt in to approximate weather through the fixed forecast/archive endpoints. |
| Nominatim geocoding | Blocked | Retain saved places, coordinates and stored labels; no new provider lookups. |
| API basemaps and Iconify | Blocked | Use bundled assets and route geometry; these API providers cannot be enabled. |
| Gateway maps | Disabled | Separately enable the gateway's fixed OpenFreeMap capability. |
| S3-compatible backups | Disabled | Configure a destination and credentials in Settings > Backups. |
| GitHub release checks | Disabled | Maintainers must configure the repository URLs before enabling browser checks. |

The application uses local/system font fallbacks and packaged vehicle artwork.
Existing trip data, weather history, stored labels and local artwork remain
available when a connection is disabled. OIDC authentication has separate
configuration and controls; see [privacy](../privacy.md).

## Weather and outside temperature

An administrator or super user can enable Open-Meteo in External Connections.
Only the fixed forecast and archive endpoints, approximate coordinate precision,
and requests without provider credentials are allowed. Custom/self-hosted URLs,
exact-coordinate mode and private-network endpoints are rejected. A fresh
installation starts disabled; enabling weather is an explicit data-sharing choice.
Disabling it pauses provider work without removing stored weather history.

The weather workflow selects the trip start, end and 15-minute samples locally.
Provider coordinates are rounded to two decimal places, deduplicated and shuffled;
each request sends one approximate location and a date range. Trips less than
five days old use the forecast endpoint; older trips use the archive endpoint.
Rivian credentials and account/vehicle identifiers are not sent to Open-Meteo.
The provider still receives ordinary network metadata from your server.

The installation shares a budget of 200 weather requests per UTC day, with at
least five seconds between admitted requests. Responses are limited to 64 KiB.
Missing data or an exhausted budget leaves weather incomplete rather than
presenting an estimate as a measurement. Stored vehicle temperatures take
precedence where available; derived trip averages retain their source labels.

Save the enabled connection before using **Test with synthetic data**. The test
uses a fixed public location and the same request budget; it does not send a real
trip. A disabled connection, unavailable provider or exhausted budget can prevent
a test. Existing incompatible saved settings stay inactive until replaced with a
supported configuration; startup does not recreate custom endpoints or secrets.

## Geocoding

Nominatim and custom/self-hosted geocoding are blocked by the runtime policy.
Enabling saved provider flags or using the provider test cannot bypass the guard.
Trip coordinates, saved places and stored address labels remain available, but
new remote address searches and reverse-geocoding requests are unavailable.

## Basemaps

The standard API installation shows stored routes on a neutral background.
Its basemap providers, custom XYZ URLs and enable/test controls are blocked.

The optional private gateway can enable fixed OpenFreeMap cartography with
`ENABLE_FREE_MAPS=true`. This is separate gateway configuration, not an API
provider switch. Settings reports the gateway provider as enabled and read-only.
It serves public map areas through the gateway while private trip geometry stays
within the application. See [free street maps](./private-origin-gateway.md#optional-free-street-maps)
for configuration, provider traffic, limits and disabling the capability.
The setup script does not deploy or configure this gateway.

A missing map provider does not remove trip geometry. If a configured gateway
map fails, the map keeps its route and presents the corresponding configuration
or tile failure and retry action. See [Frontend error observability](../architecture/frontend-error-observability.md).

## Local provider caches

Stored cache entries can outlive the provider setting that created them. Their
presence does not mean a blocked provider can make new requests. External
Connections reports available cache counts and storage use; administrators can
purge eligible cache entries. Purging geocoding lookup records retains addresses
attached to trips, charge sessions or saved places.

API cache counters do not measure gateway map traffic. Inspect gateway usage for
that separately configured service. API basemap cache limits and expiry still
apply to retained entries, but do not enable the blocked provider path.

## Other connections

- **Iconify:** remote catalog and asset requests are blocked. Bundled icons and
  supported saved selections remain available.
- **Vehicle artwork:** new remote artwork downloads are blocked. The browser uses
  local cached artwork or the packaged fallback for the vehicle model.
- **S3-compatible backups:** status appears here; destination, credentials and
  enablement are managed in **Settings > Backups**. Local backups remain separate.
- **Rivian account:** managed in **Settings > Vehicles**. Application policy rejects
  vehicle-changing requests. Provider tokens themselves are not provider-enforced
  read-only credentials; protect the server and its external keys.

## GitHub Releases

Release checks are disabled by default. The checked-in release-page and API URLs
still use `your-org/r-the-drive`; they are repository templates, not a published
release source. Maintainers must configure the canonical repository and publish
reviewed releases before enabling this feature. The setup script neither
publishes releases nor installs automatic updates.

Once configured and enabled by an administrator or super user, each browser
checks on first open and at the selected hourly, daily, weekly or monthly
interval while the app is open. A monthly interval is 30 days. Requests go from
the browser to GitHub without application account or vehicle data; GitHub receives
ordinary browser network metadata. The last attempt and result stay in that
browser. An available-release link opens GitHub; it does not install an update.

## Destination checks and bounded caches

Supported weather requests resolve and pin approved public addresses before
connecting, reject redirects and count actual response bytes. There is no
weather or geocoder private-CIDR exception. OIDC and S3 use their separate
operator-controlled destination policies.

Blocked map, geocoding and media settings have no environment opt-out. Supporting
new destinations requires a reviewed implementation with request-time address
validation, redirect denial and bounded responses. Do not disable the guard to
make an unsupported provider work.
