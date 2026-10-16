import { readFileSync } from 'node:fs';
import { expect, test, type Page } from '@playwright/test';

const dashboard = JSON.parse(
  readFileSync(
    new URL('../../../../packages/dashboards/src/defaults/dashboard.json', import.meta.url),
    'utf8'
  )
);
const vehicle = {
  id: 'vehicle-1',
  user_id: 'e2e-user',
  rivian_vehicle_id: 'synthetic-rivian',
  model: 'R1T',
  display_name: 'Fixture R1T',
  membership_role: 'owner',
  is_demo: false,
  images: null,
  created_at: '2026-01-01T00:00:00Z',
};

async function installFixture(page: Page, locked: boolean | null) {
  const mutations: string[] = [];
  const status = {
    vehicle_id: vehicle.id,
    doors_locked: locked,
    door_rear_left_locked: locked,
    door_front_left_locked: locked,
    door_rear_right_locked: locked,
    door_front_right_locked: locked,
    closure_tailgate_locked: locked,
    closure_frunk_locked: locked,
    battery_level: 75,
    battery_limit: 80,
    range_miles: 200,
  };
  await page.routeWebSocket('**/v1/vehicles/live**', (socket) => socket.close());
  await page.route('**/*', async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    if (url.hostname !== '127.0.0.1') return route.abort();
    if (!/^\/v[12]\//.test(url.pathname)) return route.continue();
    if (!['GET', 'HEAD'].includes(request.method())) mutations.push(url.pathname);
    const responses: Record<string, unknown> = {
      '/v1/auth/bootstrap': {
        access_token: 'eyJhbGciOiJub25lIn0.eyJzdWIiOiJlMmUtdXNlciJ9.',
        expires_in: 3600,
        default_vehicle_id: vehicle.id,
      },
      '/v1/auth/me': {
        user_id: 'e2e-user',
        email: 'fixture@example.invalid',
        role: 'user',
        default_vehicle_id: vehicle.id,
      },
      '/v1/auth/preferences': {
        units: { mode: 'imperial', distance_unit: 'miles', temperature_unit: 'fahrenheit' },
      },
      '/v1/vehicles': { vehicles: [vehicle] },
      '/v1/vehicles/vehicle-1/status': status,
      '/v1/vehicles/vehicle-1/images': { all: [] },
      '/v1/dashboards': [dashboard],
      '/v1/dashboards/by-slug/dashboard': dashboard,
      '/v1/charts/effective': [],
      '/v1/metrics/catalog': { metrics: [] },
      '/v1/metrics/batch': { values: [], series: [] },
      '/v2/auth/preferences/theme': { mode: 'system', light_theme_id: null, dark_theme_id: null },
      '/v1/metrics/value': { value: null, ts: null },
    };
    return route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(responses[url.pathname] ?? {}),
    });
  });
  return mutations;
}

for (const width of [1280, 390]) {
  for (const [label, locked] of [
    ['Locked', true],
    ['Unlocked', false],
    ['Locks unknown', null],
  ] as const) {
    test(`${label} is telemetry only at ${width}px`, async ({ page }) => {
      await page.setViewportSize({ width, height: 844 });
      const mutations = await installFixture(page, locked);
      await page.goto('/');
      const name = locked === null ? 'Vehicle lock status unavailable' : locked ? 'Vehicle locked' : 'Vehicle unlocked';
      const indicator = page.getByRole('img', { name, exact: true });
      await expect(indicator).toBeVisible();
      mutations.length = 0;
      await indicator.click();
      await expect(page.getByRole('dialog')).toHaveCount(0);
      await expect(
        page.getByRole('button', { name: /^(lock|unlock)( vehicle| doors)?$/i })
      ).toHaveCount(0);
      await page.evaluate(
        () => new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve())))
      );
      await expect(indicator).toHaveAccessibleName(name);
      expect(mutations).toEqual([]);
    });
  }
}
