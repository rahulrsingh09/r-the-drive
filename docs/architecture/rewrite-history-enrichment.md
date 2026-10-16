# History enrichment transformations

R:The Drive derives location labels, compact routes and weather estimates from
saved trips and telemetry. These services preserve PANDA/panda storage and the
current R interface. This rewrite retains the project's GPL and NOTICE
attribution; it does not claim independent or exclusive authorship.

Documentation impact: Documentation site update required. The integration owner
must register this page in the shared documentation sidebar.

## Data flow and limits

`trip_enrichment` resolves endpoint coordinates into optional geofence and address
matches, then persists only the derived fields and owner annotations. A coordinate
pair and an address-resolution result make absence, a successful lookup and a
failed lookup explicit. Address decoding produces one normalized record before
persistence. An OSM identifier conflict refreshes labels and raw response data
without moving the original cached location; rows without OSM identifiers remain
independent records.

Full-history geofence, temperature-lookup and weather-queue passes read pages of
at most 500 rows, ordered by start time and UUID. A cursor advances before writes,
so updated eligibility cannot skip subsequent pages. The initial vehicle-scoped
row count limits work even if rows arrive during a pass. Pages release their pool
connection before any nested lookup or write; a one-connection pool is supported.
Address reconciliation still selects the newest 100 candidates; coordinate repair
still selects the newest 500. These two passes retain their existing starvation
behavior when unresolved recent rows remain eligible. Full-history passes do not
promise a repeatable-read snapshot during concurrent edits; subsequent runs
reconcile changes.

`trip_routes` removes nonfinite coordinates, the `(0,0)` sentinel and consecutive
duplicates. Up to 256 points pass through unchanged except for conversion from
`(latitude, longitude)` into `[longitude, latitude]`. Larger routes are projected
into metres using the mean latitude. Simplification uses an explicit segment
stack, preserving the original first-furthest tie rule and strict tolerance
comparison. At most 512 tolerance doublings and twelve refinements bound the
search. Working memory scales with the input route; the output contains at most
256 points, including the endpoints.

`weather_enrichment` first rejects negative or greater-than-one-day trip windows.
It builds at most 97 local sample times: the start, every fifteen-minute boundary,
and the end. Route and temperature query results stream into nearest-value slots,
so retained telemetry memory is bounded by the sample count. Route rows still
match the vehicle, inclusive trip window, and either this trip ID or no trip ID.
Temperature rows still match the vehicle and time window regardless of trip ID.
No input telemetry rows or driving measurements are rewritten.

The weather plan assigns measured Celsius values first. A measurement within
450 whole seconds wins; the first row wins a tie. Unresolved targets share coarse
two-decimal weather cells. Requests retain the one-location limit, shuffle,
connection checks, heartbeat and delay. Provider payloads are mapped to the
nearest usable hourly value within 3600 whole seconds. The final estimate uses
trapezoidal elapsed-time weighting and records `vehicle`, `open_meteo` or `mixed`.

## Compatibility and persistence

- Missing latitude or longitude triggers whole-pair repair from the nearest
  vehicle fix within an inclusive ten-minute window. Existing `(0,0)` endpoints
  do not trigger coordinate repair. Automatically located charge sessions can be
  repaired; manually cleared locations remain untouched.
- Owner annotations use the oldest owner membership. Existing trip/session
  location values win `COALESCE` writes, while new annotation matches take
  precedence over old annotations. Other users' geofences do not participate.
- The shared address cache selects the nearest match strictly under 100 metres.
  Cache errors fall through to the existing provider seam. A disabled provider,
  empty result or provider error returns no address. One trip can increment both
  `filled` and `failed` when only one missing endpoint resolves.
- History weather enqueue resets failed or unavailable jobs to pending with zero
  attempts, and skips other existing statuses. Single-trip enqueue preserves
  succeeded jobs and does not reset attempts. Worker claim, recovery, retry,
  backoff and daily-budget behavior remain unchanged.
- Derived weather samples and the trip summary are replaced in one transaction;
  an insert failure restores the previous cache and summary. Only provider
  estimates are stored as samples. Hourly response data is local to one plan;
  this module does not implement a persistent HTTP cache shared between trips.
- Existing validation differences are retained. History location checks reject
  absent pairs and `(0,0)` but do not reject all nonfinite or out-of-range values.
  Route previews reject nonfinite values but retain finite out-of-range values.
  Vehicle temperature selection retains nonfinite values. The existing private
  weather seam validates actual outbound coordinates and provider temperatures.

The reconciliation worker, weather worker, private weather policy, Nominatim
scheduler and outbound client remain the authorities for consent, provider
admission, timeouts and errors. This service introduces no configuration, route,
permission, schema or provider-policy change.

## Verification and integration dependencies

Module-local characterization covers synthetic interrupted trips, sparse GPS,
partial coordinate pairs, owner separation, cache hits and disabled-provider
misses, partial address failures, 100/500-row boundaries, queue replay,
transaction rollback, invalid/null temperatures and coordinates, ties, units and
elapsed-time weighting. Frozen route fingerprints and a separate comparison of
96 synthetic routes establish exact geometry parity against the pre-rewrite
implementation. Weather payload tests use synthetic in-memory JSON. Database
fixtures use disposable TimescaleDB and Redis on an internal network without
published ports; they never contact live providers.

Geospatial queries here and in the shared geofence model call `ll_to_earth` and
`earth_distance` without schema qualification. The baseline installs these
extensions in `panda`, while the migration coordinator restores `public` as its
search path. Database parity tests explicitly configure
`panda,timeseries,public`; the integration owner must resolve the shared
search-path dependency. It is not silently repaired in this scoped rewrite.

See [backend data flow](./backend-data-flow.md) for the adjacent ingestion and
provider boundaries.
