import type { Page, Route } from '@playwright/test';
import { readFileSync } from 'node:fs';

export async function installFeatureFixture(page: Page, options: { persistMutations?: boolean; role?: 'user' | 'admin' | 'super_user' } = {}) {
  const createdCharts: Array<{
    config: {
      placements: Array<{ dashboardSlug: string }>;
      x?: { kind?: string; field?: { field?: string } };
      series?: Array<{ label?: string; yAxis?: string }>;
    };
  }> = [];
  const chartSeeds = readJson<Array<{ slug: string; name: string; description: string; enabled: boolean; definition: unknown }>>('../../../../packages/dashboards/src/charts/defaults/defaults.json');
  const sourceManifests = readJson<unknown[]>('../../../../packages/dashboards/src/charts/sources/sources.json');
  const dashboards = ['dashboard', 'battery', 'charging', 'efficiency', 'trips'].map((slug) => readJson<Record<string, unknown>>(`../../../../packages/dashboards/src/defaults/${slug}.json`));
  const savedLayout = { ...structuredClone(dashboards[0]), id: '11111111-2222-4333-8444-555555555555', slug: 'saved-layout', name: 'Saved layout', ownerId: 'aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee', isDefault: false };
  dashboards.push(savedLayout);
  const dashboardRequests: string[] = [];
  const assignmentRequests: string[] = [];
  page.on('request', request => {
    const path = new URL(request.url()).pathname;
    if (request.method() !== 'GET' && path.includes('/dashboards')) dashboardRequests.push(`${request.method()} ${path}`);
    if (request.method() !== 'GET' && path.includes('/placements')) assignmentRequests.push(`${request.method()} ${path}`);
  });
  let entries = chartSeeds.map((chart, index) => ({
    effective: { id: `chart-${index + 1}`, ownerId: null, slug: chart.slug, name: chart.name, description: chart.description, isDefault: true, isLocked: false, isEnabled: chart.enabled, baselineRevision: 5, config: chart.definition },
    origin: 'system',
    permissions: { read: true, edit: true, duplicate: true, reset: false, restore: false, delete: false, lock: false },
  }));

  await page.route('**/v2/**', route => {
    const path = new URL(route.request().url()).pathname;
    return json(route, path.endsWith('/preferences/theme')
      ? { schemaVersion: 2, mode: 'dark', selection: { kind: 'builtin', themeId: 'classic' } }
      : { schemaVersion: 2, registryHash: 'test', builtins: [], customThemes: [] });
  });
  await page.routeWebSocket('**/v1/vehicles/live**', (socket) => socket.close());
  await page.route('**/v1/**', async (route) => {
    const path = new URL(route.request().url()).pathname;
    if (path === '/v1/auth/bootstrap') return json(route, authTokens());
    if (path === '/v1/auth/me') return json(route, { user_id: 'e2e-user', email: 'editor@panda.test', role: options.role ?? 'user', default_vehicle_id: 'vehicle-1' });
    if (path === '/v1/auth/preferences') return json(route, { units: unitPreferences() });
    if (path === '/v1/vehicles') return json(route, { vehicles: [testVehicle()] });
    if (path === '/v1/dashboards') return json(route, dashboards);
    if (path.startsWith('/v1/dashboards/')) return json(route, savedLayout);
    if (path === '/v1/charts' && route.request().method() === 'POST') {
      const body = route.request().postDataJSON() as { config: { placements: Array<{ dashboardSlug: string }> } };
      createdCharts.push(body);
      const created = { ...body, id: 'personal-override', ownerId: 'e2e-user', isDefault: false, isLocked: false, baselineRevision: null };
      if (options.persistMutations) {
        const slug = (body as { slug?: string }).slug;
        entries = entries.map((entry) => entry.effective.slug === slug ? {
          ...entry, effective: created as typeof entry.effective, personalOverride: created,
          origin: 'override', permissions: { ...entry.permissions, reset: true, delete: true },
        } : entry);
      }
      return json(route, created);
    }
    if (path.startsWith('/v1/charts/') && route.request().method() === 'PUT') {
      const id = path.split('/').at(-1);
      const body = route.request().postDataJSON() as Record<string, unknown>;
      let updated: unknown = body;
      entries = entries.map((entry) => {
        if (entry.effective.id !== id) return entry;
        updated = { ...entry.effective, ...body };
        return { ...entry, effective: updated as typeof entry.effective, personalOverride: updated, origin: 'override' };
      });
      return json(route, updated);
    }
    if (path === '/v1/charts/effective') {
      const placement = new URL(route.request().url()).searchParams.get('dashboard_slug');
      return json(route, entries.map((entry) => entry.effective).filter((chart) => chart.isEnabled && chart.config.placements.some((candidate) => candidate.dashboardSlug === placement)));
    }
    if (path === '/v1/charts') return json(route, entries);
    if (path === '/v1/chart-sources') return json(route, sourceManifests);
    if (path === '/v1/metrics/catalog') return json(route, { metrics: [
      { id: 'battery_level', label: 'Battery Level', unit: '%', kind: 'percent', source: 'telemetry', supports_series: true, default_aggregation: 'avg' },
      { id: 'energy_charged', label: 'Energy Charged', unit: 'kWh', kind: 'energy', source: 'summary', supports_series: true, default_aggregation: 'sum' },
    ] });
    if (path === '/v1/metrics/series') return json(route, []);
    if (path === '/v1/battery/mileage') return json(route, [
      { ts: '2026-08-01T00:00:00Z', odometer_mi: 18800, usable_kwh: 111.2, range_mi: 331, projected_max_range_mi: 334, degradation_pct: 2.1 },
      { ts: '2026-08-15T00:00:00Z', odometer_mi: 19000, usable_kwh: 111.5, range_mi: 329, projected_max_range_mi: 333, degradation_pct: 2.0 },
      { ts: '2026-08-29T00:00:00Z', odometer_mi: 19200, usable_kwh: 111.1, range_mi: 330, projected_max_range_mi: 332, degradation_pct: 2.2 },
    ]);
    if (path.startsWith('/v1/battery/') || path.startsWith('/v1/efficiency/')) return json(route, []);
    if (path.endsWith('/status')) return json(route, {});
    if (path.endsWith('/images')) return json(route, { all: [] });
    return json(route, {});
  });
  return { createdCharts, savedLayout, dashboardRequests, assignmentRequests };
}

function readJson<T>(path: string): T { return JSON.parse(readFileSync(new URL(path, import.meta.url), 'utf8')) as T; }
function authTokens() { return { access_token: 'test-access-token', expires_in: 3600, default_vehicle_id: 'vehicle-1' }; }
function testVehicle() { return { id: 'vehicle-1', user_id: 'e2e-user', rivian_vehicle_id: 'rivian-vehicle-1', vin: null, model: 'R1T', year: 2025, trim: 'Adventure', color: 'Forest Green', battery_capacity_kwh: 135, display_name: 'Test R1T', created_at: '2026-01-01T00:00:00Z', images: null, membership_role: 'owner' }; }
function unitPreferences() { return { mode: 'imperial', distance_unit: 'miles', speed_unit: 'mph', temperature_unit: 'fahrenheit', pressure_unit: 'psi', altitude_unit: 'feet', place_radius_unit: 'feet', efficiency_display: 'distance_per_energy' }; }
function json(route: Route, body: unknown) { return route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) }); }
