# Backend Data Flow

## Audience

Backend contributors changing auth, ingestion, storage, or route behavior.

## Source Of Truth

This document is canonical for the high-level backend flow. Update it when the API runtime path or ingestion architecture changes materially.

## Database function resolution

The shared application pool sets each connection's search path to
`pg_catalog, panda, public, pg_temp`. Built-ins resolve first, the trusted
application schema provides `cube`/`earthdistance` functions, types and operators,
and `public` retains the existing TimescaleDB helpers. Temporary types resolve
last. Neither `$user` nor a caller-supplied schema is added. The supported schema
setup does not grant `CREATE` on `panda` or `public` to `PUBLIC`; these schemas
must remain unavailable for object creation by untrusted roles.

The migration pool is separate and retains its explicit `public` search path.
Application connections opened before migration and connections added later
receive the same application configuration. This changes no schema, extension
ownership, role attribute or history privilege.

Qualifying scalar calls can work with earthdistance 1.2, whose SQL bodies bind
their dependencies at creation. Retained 1.1 installations also have nested
function/type lookups in string bodies, and application geofence queries use
extension operators. Configuring the shared connection covers these existing
queries without changing extension definitions or requiring fixture-only
search-path overrides. The pool configuration does not install or upgrade
extensions.

## Flow Summary

1. User authenticates with Panda through the auth routes.
2. Vehicle credentials and session state are stored by the API.
3. Per-vehicle ingestion workers maintain Rivian connectivity through WebSocket and supporting poll flows, with a watchdog that restarts a collector if the WebSocket stream goes silent while still holding the worker lock. The authenticated socket carries both `vehicleState` and the documented `chargingSession` subscription. A small one-shot `GetVehicleState` query at startup supplies a baseline for fields that have not changed since subscription.
4. Parsed telemetry updates a canonical `vehicle_latest_status` row using per-field Rivian timestamps so older partial payloads cannot overwrite fresher SoC, range, charge-state, or odometer values.
5. Supporting poll flows reconcile charging sessions and post-session history into canonical `charge_sessions`, preserving telemetry-backed windows as the public session timeline while storing Rivian aliases and API-only history as enrichment evidence. Canonical `vehicleState` owns charge lifecycle and UUID identity. Fresh, fixture-proven Parallax fields enrich power, curve, energy-breakdown, and time estimates; meaningful legacy `chargingSession` values update the provisional active-session projection before canonical finalization. Empty, stale, or terminal legacy frames never extend the live projection, and canonical termination deletes Redis state immediately. Final cost is computed only after the session has an authoritative end time.
6. API routes expose typed data to the frontend through `packages/types` and `packages/hooks`.
   Charging frames rarely carry GPS, so a completed charge session without coordinates borrows the nearest telemetry fix (within 10 minutes of its start) before its saved place and address are matched. A six-hourly reconciler repeats this for trips and charge sessions that were missed: it recovers missing coordinates from telemetry, matches geofences, and reverse-geocodes the rest. Charge sessions whose location the user edited by hand (`location_override_mode` other than `automatic`) are never touched.
7. Completed trips enqueue an idempotent weather-enrichment job. The worker samples the exact route at endpoints and 15-minute intervals, derives rounded provider cells, batches Open-Meteo requests, stores `trip_weather_samples`, and updates the time-weighted `trips.outside_temp_c` summary used by trip and efficiency APIs.

Runtime feed health is separate from telemetry freshness. Authentication errors,
collector failures, degraded subscriptions, and a silent WebSocket can make the
feed unhealthy. Older battery, range, or charging timestamps are reported as
field freshness diagnostics instead; parked vehicles are not expected to emit
continuous trip or charging telemetry.

## Trip capture and worker recovery

Migration `0028_active_trip_checkpoints.sql` adds versioned detector snapshots,
a pending-completion queue, and collector heartbeat/persistence diagnostics.
Each accepted trip sample advances the checkpoint before telemetry processing
continues. When the detector closes a trip, the same transaction saves the
completed payload to `pending_trip_completions` and advances the checkpoint.
The worker retains the candidate in memory and retries failed checkpoint writes
at 1–30 second intervals without processing another trip sample. Acquisition
channels remain bounded; this backpressure does not provide an unlimited outage
buffer.

The completion consumer runs at startup, before credentials or Redis are needed,
and every 15 seconds during collection. In one transaction it inserts the trip
with its original UUID, enqueues weather enrichment, and removes the completion.
A conflicting UUID must have matching vehicle/start/end identity; retry never
overwrites an existing trip or user edits. Failed completion writes retain the
queue row and report a persistence error. The existing 0.1-mile minimum still
applies. Optional address/geofence matching runs through the existing periodic
location reconciler after capture; geocoding availability cannot prevent the
canonical trip write.

Restart rehydrates the detector before starting acquisition. Replayed samples
at or before the checkpoint's observation watermark cannot advance it again.
Recovery clears energy/elevation integration boundaries and cached gear anchors,
and starts signal fusion afresh. If the last observation is over five minutes
old at recovery or at the first new sample, the old fragment ends at that last
observation, with its original UUID and observed endpoints. A later trip starts
from fresh observations. No samples, intermediate route points, or energy
integration are synthesized across downtime. Short gaps can still conceal a
stop/restart; heuristic boundaries cannot prove that two observations are the
same real-world trip.

The supervisor checks exited tasks every second and restarts them with
exponential delays from 5 seconds to 5 minutes. Five minutes of worker lifetime
resets the failure count. Explicit stop, shutdown, and command-channel closure
remove desired workers and cancel retries; shutdown waits up to five seconds
before aborting a stuck task. Acquisition tasks abort when their worker drops,
and the advisory-lock connection closes instead of returning a held session
lock to the pool. A broken checkpoint connection must reacquire ownership and
verify the saved snapshot before retrying.

The authenticated vehicle health response includes `trip_capture`: `ready`,
`collector_heartbeat_at`, `checkpoint_at`, `pending_completions`, and
`persistence_error`. Readiness requires an authorized, connected collector, a
heartbeat within 60 seconds, an existing checkpoint, and no pending completion
or persistence error. It is evidence of current collection/persistence, not a
promise of every trip. `/health` retains its Redis-backed liveness behavior;
upstream outages must not cause application restart storms.

Snapshots only cover successfully committed observations. Samples never
delivered by Rivian, channel overflow/disconnection under backpressure, and
uncommitted memory lost when the process stops while the database is unavailable
cannot be recovered by this mechanism. There is no disk spool independent of
the database. Previously committed trips are never rebuilt or deleted.

## Telemetry storage and enrichment

Telemetry is written to the `timeseries.telemetry` hypertable. The
`telemetry_1min` continuous aggregate incrementally materializes the prior
seven days once an hour, ending five minutes before the present. It remains a
real-time aggregate, so queries include the unmaterialized recent tail from
the hypertable. This keeps active dashboards and charge curves current without
running a refresh every five minutes. Do not stretch this policy to 12 hours or
daily: doing so makes dashboard reads carry an increasingly large raw-data
tail. `odometer_daily` has a separate hourly, materialized-only policy.

Optional outbound services are governed by `external_connection_settings`, not environment variables. Weather and Nominatim execute on the server. Basemap and Iconify browser requests terminate at authenticated same-origin proxy routes. Custom endpoints are validated before storage, secrets are age-encrypted and write-only, and disabling a provider is enforced at the shared service seam.

The private fork disables these optional API provider paths. Its Cloudflare
gateway may independently opt into fixed OpenFreeMap cartography with
`ENABLE_FREE_MAPS=true`: it adapts successful authenticated map configuration
responses and serves Access-protected public map resources directly, without
routing tile bytes through the API. No private trip data moves into that map
proxy. See the [gateway contract](../guides/private-origin-gateway.md).

Parallax collection runs as an integrated, isolated Tokio acquisition subsystem inside each production
vehicle worker. It opens its own allowlisted GraphQL WebSocket and writes normalized,
typed readings to the `timeseries.parallax_*` tables. A bounded channel also passes validated R2-relevant power, GNSS, odometer, closure, tire, and cabin readings to the canonical worker, where they enter latest status and telemetry history. Parallax frames cannot create or end charge sessions. The collector never stores raw payloads, network identifiers, or credentials. Its failure cannot stall
the main telemetry worker. For R2 and legacy R2S models, trip detection joins recent power state with sparse GNSS fixes and may infer speed after two plausible moving segments; invalid, stale, and jumping fixes are rejected. The original telemetry values remain unmodified in storage. The API reads the normalized tables for the Health
page and Rivian-reported Parked Energy panel; the existing Phantom Drain
battery-change estimate remains an independent derived data source.
The subsystem is integrated and enabled by default, shares only the latest canonical active-session
context, and can be disabled for emergency rollback with `PARALLAX_ENABLED=false`.

## Major Backend Areas

- `apps/api/src/routes`
  Public HTTP surface.
- `apps/api/src/ingestion`
  Rivian auth integration, WebSocket/poll workers, parser, detector logic.
- `apps/api/src/services`
  Shared backend business logic.
- `apps/api/src/models`
  DB-facing types and helpers.
- `apps/api/migrations`
  Schema evolution.

## Operational Rules

- New env vars must be reflected in `compose/.env.full.example`, the short Compose template when needed, and any relevant user-facing docs.
- New routes or route removals must update the relevant developer docs and any public-facing API references.
- Changes to auth, ingestion, or backup behavior must update runbooks if maintainers will need new recovery steps.

## Adjacent Docs

- [`../rivian-auth.md`](../rivian-auth.md)
- [`../api-access.md`](../api-access.md)
- [`../security.md`](../security.md)
- [`overview.md`](./overview.md)
