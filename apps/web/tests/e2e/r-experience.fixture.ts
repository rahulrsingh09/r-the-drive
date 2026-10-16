// Synthetic API responses; no live vehicle or account is used.
import { createAdminFixture } from './r-admin.fixture.ts';
import { createAnalysisFixture } from './r-analysis.fixture.ts';
import type { Page, WebSocketRoute } from '@playwright/test';
import type { VehicleStatus } from '@panda/types';
import { readFileSync } from 'node:fs';

export async function installRFixture(page: Page, options: {
  mode?: 'light' | 'dark';
  timezone?: string;
  populatedAdmin?: boolean;
  authenticated?: boolean;
  secondVehicle?: boolean;
  missingSensors?: boolean;
  missingSoftware?: boolean;
  statusOverrides?: Partial<VehicleStatus>;
  populatedTrip?: boolean;
  missingTripSpeed?: boolean;
  statusDelay?: number;
  unknownConfiguration?: boolean;
  palette?: 'classic' | 'rad';
  populatedAnalytics?: boolean;
  tripCount?: number;
  odometer?: number;
  vehicleName?: string;
  role?: 'user' | 'super_user';
} = {}) {
  const writes: string[] = [];
  const metricRequests: Record<string, unknown>[] = [];
  const unsupportedRequests: string[] = [];
  const liveSockets = new Set<WebSocketRoute>();
  const chartFavorites: Record<string, string> = {};
  let mode: 'light' | 'dark' | 'system' = options.mode ?? 'light';
  let timezone = options.timezone ?? 'UTC';
  const readJson = (path: string) => JSON.parse(readFileSync(new URL(path, import.meta.url), 'utf8'));
  const dashboards = ['dashboard', 'trips', 'charging', 'efficiency', 'battery'].map((slug) =>
    readJson(`../../../../packages/dashboards/src/defaults/${slug}.json`));
  const chartDefinition = readJson('../../../../packages/dashboards/src/charts/defaults/defaults.json')
    .find((item: { slug: string }) => item.slug === 'soc-history');
  const chart = {
    id: 'chart-1', ownerId: '33333333-3333-4333-8333-333333333333', slug: 'sample-battery', name: 'Battery history',
    isDefault: false, isLocked: false, isEnabled: true, config: chartDefinition.definition,
  };
  const theme = {
    themeId: 'theme-1', name: 'My appearance', baseThemeId: 'classic', publishedRevision: 1,
    publishedDefinition: { theme: 'classic' }, retiredAt: null, etag: '"preview-theme"',
    revisions: [{ revision: 1, definition: { theme: 'classic' }, definitionHash: 'preview',
      createdAt: '2026-10-08T00:00:00Z', publishedAt: '2026-10-08T00:00:00Z' }],
  };
  const vehicle = {
    id: 'owner-vehicle', user_id: '33333333-3333-4333-8333-333333333333', rivian_vehicle_id: 'fixture-r2',
    vin: null, model: 'R2', year: null, trim: 'Performance',
    color: options.unknownConfiguration ? null : 'Catalina Cove',
    wheel_option: options.unknownConfiguration ? null : '21" Liquid Tungsten All-Season',
    interior_color: options.unknownConfiguration ? null : 'Black Crater Signature',
    battery_capacity_kwh: 88, is_demo: false, display_name: options.vehicleName ?? 'Your Rivian',
    created_at: '2026-01-01T00:00:00Z', images: null, membership_role: 'owner',
  };
  const adminFixture = options.populatedAdmin ? createAdminFixture(vehicle.id) : null;
  const analysisFixture = options.populatedAdmin ? createAnalysisFixture(dashboards, chart, readJson('../../../../packages/dashboards/src/charts/defaults/defaults.json'), options.role === 'super_user') : null;
  const unitPreferences = {
    mode: 'imperial', distance_unit: 'miles', speed_unit: 'mph', temperature_unit: 'fahrenheit',
    pressure_unit: 'psi', altitude_unit: 'feet', place_radius_unit: 'feet', efficiency_display: 'distance_per_energy',
  };
  const trip = (offset = 0) => ({
    id: `trip-${offset + 1}`, vehicle_id: vehicle.id, started_at: '2026-10-08T17:00:00Z',
    ended_at: '2026-10-08T17:30:00Z', distance_mi: 20, duration_min: 30, energy_used_kwh: 6.4,
    efficiency_wh_mi: 320, max_speed_mph: 50, drive_mode: 'all_purpose', soc_start: 75, soc_end: 68,
    start_place: `Drive ${offset + 1}`, end_place: 'Home',
  });
  const charge = {
    id: 'charge-1', vehicle_id: vehicle.id, location_name: 'Home', started_at: '2026-10-08T04:00:00Z',
    ended_at: '2026-10-08T06:00:00Z', energy_added_kwh: 20, duration_min: 120, cost_usd: null,
    charger_type: 'ac', soc_start: 40, soc_end: 65, peak_power_kw: 11,
  };
  const tags = [{ id: 'tag-1', name: 'Daily commute', color: 'series-02' }, { id: 'tag-2', name: 'Weekend', color: 'series-03' }];
  const assignedTags = new Map<string, string[]>();
  const trips = Array.from({ length: options.tripCount ?? (options.populatedAnalytics ? 3 : 1) }, (_, index) => ({
    ...trip(index), start_place: ['Home', 'Lake Washington', 'Office'][index], end_place: ['Office', 'Home', 'Home'][index],
    ...(options.tripCount ? {
      started_at: `2026-10-08T${String(6 + Math.floor(index / 2) % 16).padStart(2, '0')}:${index % 2 ? '30' : '00'}:00Z`,
      start_place: `Route ${String(index + 1).padStart(2, '0')} · Home`,
      end_place: ['Office', 'Lake Washington', 'Trailhead', 'Market', 'Waterfront'][index % 5],
      distance_mi: 15 + index * 3,
      efficiency_wh_mi: 250 + index * 10,
      energy_used_kwh: (15 + index * 3) * (250 + index * 10) / 1000,
    } : {}),
  }));
  const days = Array.from({ length: 7 }, (_, index) => {
    const day = `2026-10-${String(index + 1).padStart(2, '0')}`;
    return { day_local: day, day_start: `${day}T00:00:00Z`, total_energy_kwh: [24, 37, 18, 53, 42, 31, 22][index]!, session_count: 1 };
  });
  await page.routeWebSocket('**/v1/vehicles/live**', (socket) => {
    liveSockets.add(socket);
    socket.onClose(() => liveSockets.delete(socket));
    socket.onMessage(() => socket.send(JSON.stringify({ type: 'heartbeat' })));
  });
  await page.route('**/*', async (route) => {
    const url = new URL(route.request().url());
    if (!['127.0.0.1', 'localhost'].includes(url.hostname)) return route.abort();
    const path = url.pathname;
    if (!/^\/v[12]\//.test(path)) return route.continue();
    const request = route.request();
    const json = (body: unknown, status = 200, headers: Record<string, string> = {}) =>
      route.fulfill({ status, contentType: 'application/json', headers, body: JSON.stringify(body) });
    if (!['GET', 'HEAD'].includes(request.method())) {
      if (path === '/v1/metrics/batch') metricRequests.push(request.postDataJSON());
      else if (!['/v1/auth/bootstrap', '/v1/auth/refresh', '/v1/client-errors'].includes(path)) writes.push(`${request.method()} ${path}`);
    }
    if (adminFixture && await adminFixture(url, request.method(), ['GET','HEAD'].includes(request.method()) ? {} : request.postDataJSON(), json)) return;
    if (analysisFixture && await analysisFixture(url, request.method(), ['GET','HEAD'].includes(request.method()) ? {} : request.postDataJSON(), json)) return;
    if (path === '/v1/auth/bootstrap' || path === '/v1/auth/refresh') {
      return options.authenticated === false
        ? json({ error: { message: 'No session' } }, 401)
        : json({ access_token: 'eyJhbGciOiJub25lIn0.eyJzdWIiOiIzMzMzMzMzMy0zMzMzLTQzMzMtODMzMy0zMzMzMzMzMzMzMzMifQ.', expires_in: 3600, default_vehicle_id: vehicle.id });
    }
    if (path === '/v1/auth/me') return json({ user_id: '33333333-3333-4333-8333-333333333333', email: 'owner@example.test', role: options.role ?? 'user', default_vehicle_id: vehicle.id });
    if (path === '/v1/auth/config') return json({ password_login_enabled: true, oidc_enabled: false, oidc_ready: false });
    if (path === '/v1/auth/setup') return json({ setup_required: false });
    if (path === '/v1/auth/preferences') {
      if (request.method() === 'PUT') Object.assign(unitPreferences, request.postDataJSON().units);
      return json({ units: unitPreferences });
    }
    if (path === '/v2/auth/preferences/theme') {
      if (request.method() === 'PUT') {
        const next = request.postDataJSON().mode;
        if (next === 'light' || next === 'dark' || next === 'system') mode = next;
      }
      return json({ schemaVersion: 2, mode, selection: { kind: 'builtin', themeId: options.palette ?? 'classic' } }, 200, { etag: '"fixture-1"' });
    }
    if (path === '/v2/themes/catalog') return json({ schemaVersion: 2, registryHash: 'preview', builtins: [], customThemes: [theme] });
    if (path === '/v2/themes/theme-1') return json(theme);
    if (path === '/v1/charts') return json([{ effective: chart, personalOverride: chart, origin: 'personal',
      permissions: { read: true, edit: true, duplicate: true, delete: true } }]);
    if (path === '/v1/settings/external-connections') return json({ connections: [] });
    if (path === '/v1/external/basemap/config') return json({
      enabled: false, provider_preference: 'auto', resolved_provider: 'none',
      revision: 'preview', styles: [], attributions: [],
    });
    if (path === '/v1/admin/users') return json({ users: [] });
    if (path === '/v1/admin/account-invitations') return json({ invitations: [] });
    if (path === '/v1/admin/vehicles') return json({ vehicles: [vehicle] });
    if (/\/vehicles\/[^/]+\/images$/.test(path)) return json({ all: [] });
    if (/\/vehicles\/[^/]+\/idle-drain$/.test(path)) return json({ vehicle_id: vehicle.id, periods: [] });
    if (/\/vehicles\/[^/]+\/charging-networks$/.test(path)) return json([]);
    if (/\/vehicles\/[^/]+\/ingestion-diagnostics$/.test(path)) return json({
      state: 'idle', started_at: null, ends_at: null, stopped_at: null, stop_reason: null,
      event_count: 0, last_event_at: null, truncated: false,
    });
    if (/\/vehicles\/[^/]+\/raw-data$/.test(path)) return json({
      vehicle_id: vehicle.id, samples: [], total: 0, page: 1, per_page: 25, field_coverage: [],
      coverage: {
        first_event_at: null, last_event_at: null, sample_count: 0, odometer_samples: 0,
        battery_samples: 0, range_samples: 0, outside_temp_samples: 0, power_samples: 0,
        regen_samples: 0, tire_pressure_samples: 0, lock_samples: 0, software_samples: 0,
      },
    });
    if (/\/vehicles\/[^/]+\/telemetry\/lanes$/.test(path)) return json({
      vehicle_id: vehicle.id, window: {
        from: '2026-10-01T00:00:00Z', to: '2026-10-08T00:00:00Z', resolution_seconds: 60, approximate: false,
      }, spine: [], truncated: false,
      lanes: Object.fromEntries(['battery', 'drive', 'location', 'climate', 'charging', 'health']
        .map((lane) => [lane, { numeric: {}, coverage: {}, source: 'preview' }])),
    });
    if (/\/vehicles\/[^/]+\/backfill-status$/.test(path)) return json({
      vehicle_id: vehicle.id, history_backfilled_at: null, status: null,
      rivian_session_count: null, local_session_count: 0, missing_source_count: null,
    });
    if (/\/vehicles\/[^/]+\/health$/.test(path)) return json({
      vehicle_id: vehicle.id, extended_telemetry: { collector: null },
      vehicle: { name: 'Your Rivian', model: 'R2', trim: 'Performance' },
      latest: { ts: new Date().toISOString(), twelve_volt_health: null, hv_thermal_event: null },
      runtime: { last_event_at: new Date().toISOString(), worker_health: 'healthy' },
      tires: null, closures: null, software_history: [], thermal_events_30d: 0,
    });
    if (path === '/v1/admin/backups') return json({
      settings: { enabled: false, frequency: 'daily', run_at: '03:00', timezone: 'UTC', day_of_week: 0, day_of_month: 1,
        retention_count: 7, target_type: 'local', local_enabled: true, s3_enabled: false, endpoint: '', region: null,
        bucket: '', prefix: '', access_key: null, has_secret_key: false, updated_at: null },
      recent_runs: [], artifacts: [], restore_requests: [], recent_runs_total: 0, recent_runs_page: 1,
      recent_runs_per_page: 10, next_run_at: null,
      runtime_readiness: { pg_dump_available: false, run_now_allowed: false, restore_automation_available: false, reason: 'Local design preview' },
    });
    if (path === '/v1/app/version') return json({ version: 'local-preview' });
    if (path === '/v1/settings/update-check') return json({ enabled: false, frequency: 'daily' });
    if (path === '/v1/auth/preferences/chart-favorites') {
      if (request.method() === 'PUT') {
        const body = request.postDataJSON();
        chartFavorites[body.key] = body.chart_id;
      }
      return json({ chart_favorites: chartFavorites });
    }
    if (path === '/v1/auth/identities') return json({ password_configured: true, oidc_linked: false, oidc_link_available: false });
    if (path === '/v1/api-keys') return json([]);
    if (path === '/v1/api/catalog') return json({ version: 'v1', authentication: 'Bearer token', endpoints: [] });
    if (path === '/v1/settings/authentication') {
      const values = {
        oidc_enabled: false, password_login_enabled: true, oidc_auto_login: false,
        issuer_url: null, public_base_url: null, client_id: null, button_label: 'Sign in',
        scopes: 'openid profile email', token_auth_method: 'client_secret_basic',
        auto_signup: false, auto_link_verified_email: false, allowed_email_domains: [],
        required_claim_name: null, required_claim_value: null,
      };
      return json({
        ...Object.fromEntries(Object.entries(values).map(([key, value]) => [key, { value, source: 'default' }])),
        client_secret: { configured: false, source: 'default' }, last_validation_at: null, callback_url: null,
      });
    }
    if (path === '/v1/settings/timezone') {
      if (request.method() === 'PUT') timezone = request.postDataJSON().timezone;
      return json({ timezone });
    }
    if (path === '/v1/vehicles') return json({ vehicles: [vehicle, ...(options.secondVehicle ? [{
      ...vehicle, id: 'other-vehicle', model: 'R1T', display_name: 'Other vehicle',
      trim: null, color: null, wheel_option: null, interior_color: null,
    }] : [])] });
    if (/\/vehicles\/[^/]+\/status$/.test(path)) {
      if (path.includes('other-vehicle') && options.statusDelay) await new Promise((resolve) => setTimeout(resolve, options.statusDelay));
      return json({
        vehicle_id: path.includes('other-vehicle') ? 'other-vehicle' : vehicle.id,
        battery_level: path.includes('other-vehicle') ? 20 : 68, battery_limit: 80,
        charger_state: 'Disconnected', charger_status: 'chrgr_sts_not_connected',
        range_miles: 218, odometer_miles: options.odometer ?? 12486, power_state: 'ready', is_online: true,
        last_updated: new Date().toISOString(), doors_locked: true,
        drive_mode: 'all_purpose', gear_status: 'park', speed_mph: 0, altitude_m: 125,
        ota_current_version: options.missingSoftware ? null : '2026.35', ota_available_version: null, ota_status: 'idle',
        door_front_left_locked: true, door_front_right_locked: true,
        door_rear_left_locked: true, door_rear_right_locked: true,
        closure_frunk_locked: true, closure_liftgate_locked: true,
        door_front_left_closed: true, door_front_right_closed: true,
        door_rear_left_closed: true, door_rear_right_closed: true,
        closure_frunk_closed: true, closure_liftgate_closed: true,
        cabin_temp_c: 20, outside_temp_c: 15, tire_fl_psi: 45, tire_fr_psi: 45, tire_rl_psi: 45,
        tire_rr_psi: options.missingSensors ? null : 44, tire_rr_valid: options.missingSensors ? false : true,
        ...options.statusOverrides,
      });
    }
    if (path === '/v1/metrics/batch') {
      const requestBody = request.postDataJSON();
      const valueByName: Record<string, number> = { trip_miles: 1234, total_trips: 123, energy_charged: 401, avg_efficiency: 320, avg_trip_duration: 30, avg_gross_efficiency: 350, avg_outside_temp_c: 18 };
      const unitByName: Record<string, string> = { trip_miles: 'mi', energy_charged: 'kWh', avg_efficiency: 'Wh/mi', avg_trip_duration: 'min', avg_gross_efficiency: 'Wh/mi', avg_outside_temp_c: 'C' };
      return json({
        values: requestBody.metrics.map(({ metric }: { metric: string }) => ({ metric, value: valueByName[metric] ?? null, label: metric, ts: null, unit: unitByName[metric] ?? null })),
        series: options.populatedAnalytics ? requestBody.metrics.map(({ metric }: { metric: string }) => ({
          metric, points: days.map(day => ({ ts: day.day_start, value: valueByName[metric] ?? null })),
        })) : [],
        density: 'full', bucket: 'raw',
      });
    }
    if (path === '/v1/trips') {
      const offset = Number(url.searchParams.get('offset') ?? 0);
      if (options.tripCount) {
        const page = Number(url.searchParams.get('page') ?? 1);
        const perPage = Number(url.searchParams.get('per_page') ?? 15);
        const filtered = trips.filter(item => `${item.start_place} ${item.end_place}`.toLowerCase().includes((url.searchParams.get('search') ?? '').toLowerCase()));
        return json({ items: filtered.slice((page - 1) * perPage, page * perPage).map(item => ({
          ...item, tags: tags.filter(tag => assignedTags.get(item.id)?.includes(tag.id)),
        })), total: filtered.length, page, per_page: perPage });
      }
      if (options.populatedAnalytics) return json({ items: trips.map(trip => ({
        ...trip, tags: tags.filter(tag => assignedTags.get(trip.id)?.includes(tag.id)),
      })), total: trips.length, page: 1, per_page: 15 });
      return json({ items: [trip(offset)], total: 123, page: Math.floor(offset / 10) + 1, per_page: 10 });
    }
    if (options.populatedAnalytics && path.endsWith('/trip-tags/assignments')) {
      const body = request.postDataJSON();
      for (const id of body.trip_ids) assignedTags.set(id, body.mode === 'replace'
        ? body.tag_ids : [...new Set([...(assignedTags.get(id) ?? []), ...body.tag_ids])] as string[]);
      return json({ updated_trip_count: body.trip_ids.length });
    }
    if (options.populatedAnalytics && path.endsWith('/trip-tags')) return json(tags);
    if (/^\/v1\/trips\/trip-\d+\/detail$/.test(path)) {
      const columns = ['elapsed_s', 'lat', 'lng', 'altitude_m', 'speed_mph', 'power_kw', 'regen_power_kw',
        'battery_level', 'outside_temp_c', 'cabin_temp_c', 'driver_temp_c', 'hvac_active',
        'tire_fl_psi', 'tire_fr_psi', 'tire_rl_psi', 'tire_rr_psi'];
      return json({ trip: options.tripCount ? trips.find(item => item.id === path.split('/')[3]) : trip(), sample_interval_seconds: 60, samples: Object.fromEntries(columns.map((name) => [name, options.populatedTrip
          ? Array.from({ length: 31 }, (_, index) => name === 'elapsed_s' ? index * 60
            : name === 'lat' ? 47.6 + index * .001 : name === 'lng' ? -122.34 + index * .001
              : name === 'speed_mph' ? options.missingTripSpeed ? null : 20 + index : name === 'battery_level' ? 75 - index / 5 : 20)
          : []])),
        outside_temperature: { source: 'unavailable', attribution: null, samples: [] } });
    }
    if (/^\/v1\/trips\/trip-\d+$/.test(path)) return json(trip());
    if (path.startsWith('/v1/trips/trip-')) return json([]);
    if (path === '/v1/charging') return json({ items: [charge], total: 1, page: 1, per_page: 10 });
    if (path === '/v1/charging/charge-1') return json(charge);
    if (path === '/v1/charging/sessions/charge-1/curve') return json([]);
    if (path === '/v1/charging/chart-series') return json(options.populatedAnalytics ? {
      daily: days, daily_sessions: days.map((day, index) => ({
        ...day, session_id: `charge-${index + 1}`, started_at: day.day_start,
        energy_added_kwh: day.total_energy_kwh, cost_usd: index % 2 ? 12.4 : 4.2,
        charger_type: index % 2 ? 'dc' : 'ac', location_name: index % 2 ? 'Rivian charging' : 'Home',
      })),
    } : { daily: [], daily_sessions: [] });
    if (path === '/v1/charging/summary') return json({ total_energy_kwh: 401, total_cost_usd: null, session_count: 25, unknown_cost_session_count: 25, weekly: [] });
    if (path === '/v1/efficiency/summary') return json({ avg_wh_per_mi: 320, p10_wh_per_mi: 250, p90_wh_per_mi: 450, total_miles: 1234, efficiency_miles: 1100, coverage_percent: 89 });
    if (options.populatedAnalytics && path === '/v1/efficiency/by-mode') return json([
      { drive_mode: 'all_purpose', avg_wh_per_mi: 320, trip_count: 18 },
      { drive_mode: 'conserve', avg_wh_per_mi: 280, trip_count: 12 },
      { drive_mode: 'sport', avg_wh_per_mi: 380, trip_count: 5 },
    ]);
    if (options.populatedAnalytics && path === '/v1/efficiency/trend') return json(days.map((day, index) => ({
      ts: day.day_start, trip_efficiency_wh_mi: 290 + index * 18, rolling_24h_wh_mi: 315, distance_mi: 20,
    })));
    if (options.populatedAnalytics && path === '/v1/efficiency/vs-temp') return json(days.map((_, index) => ({
      temp_c_low: 8 + index * 4, temp_c_high: 12 + index * 4,
      avg_efficiency_wh_mi: 380 - index * 15, trip_count: 5, total_miles: 20 + index * 10, avg_speed_mph: 30 + index * 5,
    })));
    if (options.populatedAnalytics && path === '/v1/efficiency/by-tag') return json([
      { tag_id: 'tag-daily', tag_name: 'Daily commute', trip_count: 8, total_miles: 160, efficiency_miles: 160, avg_efficiency_wh_mi: 300, coverage: 1 },
      { tag_id: 'tag-weekend', tag_name: 'Weekend adventure', trip_count: 3, total_miles: 240, efficiency_miles: 180, avg_efficiency_wh_mi: 360, coverage: .75 },
      { tag_id: null, tag_name: 'Untagged', trip_count: 2, total_miles: 30, efficiency_miles: 30, avg_efficiency_wh_mi: 320, coverage: 1 },
    ]);
    if (options.populatedAnalytics && ['/v1/battery/soc', '/v1/battery/range'].includes(path)) return json(days.map((day, index) => ({
      ts: day.day_start, value: path.endsWith('/soc') ? 78 - index * 3 : 280 - index * 8,
    })));
    if (path.startsWith('/v1/dashboards/by-slug/')) {
      const slug = path.split('/').pop();
      const dashboard = dashboards.find((item) => item.slug === slug);
      if (dashboard) return json(dashboard);
    }
    if (/^\/v1\/dashboards\/[a-f0-9-]+$/.test(path)) {
      const dashboard = dashboards.find((item) => item.id === path.split('/').pop());
      if (dashboard) return json(dashboard);
    }
    if (path === '/v1/trips/map') return json({
      vehicle_id: vehicle.id, from: days[0]!.day_start, to: days[6]!.day_start,
      total_trips: options.populatedAnalytics ? trips.length : 0, missing_route_count: 0,
      routes: options.populatedAnalytics ? trips.map((trip, index) => ({
        trip_id: trip.id, tags: tags.filter(tag => assignedTags.get(trip.id)?.includes(tag.id)),
        coordinates: Array.from({ length: options.tripCount ? 40 : 20 }, (_, point) => options.tripCount
          ? [-122.34 + point * .0015 + Math.sin(point / 8 + index) * .004,
            47.6 + point * (.0003 + index * .000055) + Math.sin(point / 12) * .002]
          : [-122.34 + point * .003, 47.6 + point * .002 + index * .008]),
      })) : [],
    });
    if (path === '/v1/trips/tire-pressure-timeline') return json({
      vehicle_id: vehicle.id, from: days[0]!.day_start, to: days[6]!.day_start,
      samples: [], trips: [],
    });
    if (path === '/v1/dashboards') return json(dashboards);
    if (path === '/v1/charts/effective') return json([]);
    if (path === '/v1/chart-sources') return json([]);
    if (path === '/v1/metrics/catalog') return json({ metrics: [] });
    if (path === '/v1/metrics/series' || path.startsWith('/v1/battery/') || path.startsWith('/v1/efficiency/')) return json([]);
    if (path === '/v1/places') return json({ places: [] });
    if (path === '/v1/settings/basemap') return json({ provider: 'none', tile_url: null });
    unsupportedRequests.push(`${request.method()} ${path}`);
    return json({ error: { message: 'This endpoint is unavailable in the local preview.' } }, 404);
  });
  return {
    writes, metricRequests, unsupportedRequests,
    liveConnections: () => liveSockets.size,
    publishStatus: (data: Partial<VehicleStatus>) => {
      const message = JSON.stringify({ type: 'status', ts: new Date().toISOString(), data });
      for (const socket of liveSockets) socket.send(message);
    },
  };
}
