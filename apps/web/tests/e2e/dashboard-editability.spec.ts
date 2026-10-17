import { expect, test, type Page, type Route } from '@playwright/test';
import { readFileSync } from 'node:fs';
import type { DashboardConfig } from '../../../../packages/dashboards/src/schema';

const dashboard = loadDashboard('dashboard');
const battery = loadDashboard('battery');
const charging = loadDashboard('charging');
const efficiency = loadDashboard('efficiency');
const trips = loadDashboard('trips');
const chartSeeds = readJson<Array<{
  slug: string;
  name: string;
  description: string;
  enabled: boolean;
  definition: { placements: Array<{ dashboardSlug: string }> };
}>>('../../../../packages/dashboards/src/charts/defaults/defaults.json');
const chartSources = readJson<unknown[]>('../../../../packages/dashboards/src/charts/sources/sources.json');
const customDashboard: DashboardConfig = {
  ...dashboard,
  id: '11111111-1111-4111-8111-111111111111',
  slug: 'custom-dashboard',
  name: 'Custom Dashboard',
  isDefault: false,
  isLocked: false,
  ownerId: '22222222-2222-2222-2222-222222222222',
  widgets: [
    {
      id: '33333333-3333-3333-3333-333333333333',
      componentType: 'sensor',
      definitionId: 'total_miles',
      title: 'Custom miles',
      layout: { x: 0, y: 0, w: 3, h: 2 },
      options: {},
    },
    {
      id: '33333333-3333-3333-3333-333333333334',
      componentType: 'sensor',
      definitionId: 'total_miles',
      title: 'Offset miles',
      layout: { x: 5, y: 0, w: 5, h: 3 },
      options: {},
    },
    {
      id: '33333333-3333-3333-3333-333333333335',
      componentType: 'sensor',
      definitionId: 'total_miles',
      title: 'Lower miles',
      layout: { x: 1, y: 4, w: 4, h: 2 },
      options: {},
    },
  ],
};

const dashboards = new Map<string, DashboardConfig>([
  ['dashboard', dashboard],
  ['overview-fixture', { ...dashboard, slug: 'overview-fixture' }],
  ['battery', battery],
  ['charging', charging],
  ['efficiency', efficiency],
  ['trips', trips],
  ['custom-dashboard', customDashboard],
]);

const routeCases = [
  { path: '/d/overview-fixture', slug: 'dashboard', widgetId: 'd1000001-0000-0000-0000-000000000002' },
  { path: '/battery', slug: 'battery', widgetId: 'd2000002-0000-0000-0000-000000000001' },
  { path: '/charging', slug: 'charging', widgetId: 'd4000004-0000-0000-0000-000000000001' },
  { path: '/efficiency', slug: 'efficiency', widgetId: 'd3000003-0000-0000-0000-000000000001' },
  { path: '/trips', slug: 'trips', widgetId: 'd5000005-0000-0000-0000-000000000005' },
  { path: '/d/custom-dashboard', slug: 'custom-dashboard', widgetId: '33333333-3333-3333-3333-333333333333' },
] as const;

test.describe('dashboard editability in a browser', () => {
  test('renders the R1T fallback overview with tailgate and side-bin cover states', async ({ page }) => {
    await page.setViewportSize({ width: 1240, height: 760 });
    await installApiMocks(page, {
      vehicleStatus: {
        vehicle_id: 'vehicle-1',
        battery_level: 78,
        range_miles: 248,
        battery_limit: 85,
        door_rear_left_locked: true,
        door_front_left_locked: true,
        door_rear_right_locked: true,
        door_front_right_locked: true,
        closure_tailgate_locked: true,
        closure_liftgate_locked: false,
        closure_frunk_locked: true,
        side_bin_left_locked: false,
        side_bin_right_locked: true,
        side_bin_left_closed: true,
        side_bin_right_closed: false,
      },
    });
    await page.goto('/d/overview-fixture');

    const fallbackArtwork = page.locator('img[src="/vehicle-images/fallbacks/r1t/overview.webp"]').first();
    await expect(fallbackArtwork).toBeVisible();
    await expect(fallbackArtwork).toHaveAttribute('data-artwork-fallback', 'true');
    const fallbackTransform = await fallbackArtwork.evaluate((image) => image.style.transform);
    expect(fallbackTransform).toContain('translate(-50%, -50%) rotate(90deg) scaleX(');
    expect(Number(fallbackTransform.match(/scaleX\(([^)]+)\)/)?.[1])).toBeCloseTo(509 / 446);

    await expect(page.getByTitle('Tailgate lock')).toHaveClass(/left-\[4%\]/);
    await expect(page.getByTitle('Left side bin cover: closed')).toHaveClass(/left-\[36%\]/);
    await expect(page.getByTitle('Left side bin cover: closed')).toHaveClass(/top-\[24%\]/);
    await expect(page.getByTitle('Left side bin cover: closed')).toHaveClass(/text-status-positive/);
    await expect(page.getByTitle('Right side bin cover: open')).toHaveClass(/left-\[36%\]/);
    await expect(page.getByTitle('Right side bin cover: open')).toHaveClass(/top-\[76%\]/);
    await expect(page.getByTitle('Right side bin cover: open')).toHaveClass(/text-accent/);
    await expect(page.getByTitle('Tonneau lock')).toHaveCount(0);

    await page.setViewportSize({ width: 390, height: 844 });
    await page.reload();
    const leftCover = page.getByTitle('Left side bin cover: closed');
    const rightCover = page.getByTitle('Right side bin cover: open');
    await leftCover.scrollIntoViewIfNeeded();
    await expect(leftCover).toBeVisible();
    await expect(rightCover).toBeVisible();
    const [leftBox, rightBox] = await Promise.all([leftCover.boundingBox(), rightCover.boundingBox()]);
    expect(leftBox?.width).toBeGreaterThanOrEqual(24);
    expect(rightBox?.width).toBeGreaterThanOrEqual(24);
  });

  for (const routeCase of routeCases) {
    test(`${routeCase.slug} renders a fixed layout with no edit entry points`, async ({ page }) => {
      const state = await installApiMocks(page);
      await page.goto(routeCase.path + '?edit=1');
      const widget = page.locator(`[data-widget-id="${routeCase.widgetId}"]`).first();
      await expect(widget).toBeVisible();
      await expect(page.getByRole('button', { name: 'Edit dashboard', exact: true })).toHaveCount(0);
      await expect(page.getByRole('button', { name: 'Save dashboard changes' })).toHaveCount(0);
      await expect(page.getByRole('button', { name: 'Discard dashboard changes' })).toHaveCount(0);
      await expect(page.locator('[data-widget-frame="edit"], .react-resizable-handle, [data-widget-edit-control="true"]')).toHaveCount(0);
      await expect(page.getByTitle('Duplicate dashboard')).toHaveCount(0);
      await expect(page.getByTitle('Import from YAML')).toHaveCount(0);
      expect(state.layoutMutations).toEqual([]);
    });
  }
});

for (const width of [390, 1440]) {
  for (const role of ['user', 'admin', 'super_user']) {
  test(`web shell preserves exact saved placement through edit bypass and reload for ${role} at ${width}px`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width, height: 1000 });
    const state = await installApiMocks(page, { role });
    const path = `/d/custom-dashboard?dashboardId=${customDashboard.id}`;
    await page.goto(path);
    const frame = page.locator(`[data-widget-id="${customDashboard.widgets[0]!.id}"]`).first();
    await expect(frame).toContainText('Custom miles');
    await expect(frame).toBeVisible();
    await expectSavedPlacement(page, width);
    const before = await savedWidgetBoxes(page);
    await page.goto(`${path}&edit=1`);
    await expectSavedPlacement(page, width);
    await expect.poll(() => savedWidgetBoxes(page)).toEqual(before);
    await page.mouse.move(before[0]!.x + 15, before[0]!.y + 15);
    await page.mouse.down();
    await page.mouse.move(before[0]!.x + 100, before[0]!.y + 100, { steps: 8 });
    await page.mouse.up();
    expect(await savedWidgetBoxes(page)).toEqual(before);
    await expectNoLayoutControls(page);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await page.reload();
    await expectSavedPlacement(page, width);
    await expect.poll(() => savedWidgetBoxes(page)).toEqual(before);
    await expectNoLayoutControls(page);
    expect(state.layoutMutations).toEqual([]);
    await testInfo.attach('saved-layout', { body: await page.screenshot({ fullPage: true }), contentType: 'image/png' });
    await testInfo.attach('saved-placement', { body: JSON.stringify({ width, role, boxes: before, layoutMutations: state.layoutMutations }), contentType: 'application/json' });
  });
  }
}

for (const width of [390, 1440]) {
test(`web shell admin dashboard opens the exact saved view without layout mutations at ${width}px`, async ({ page }) => {
  await page.setViewportSize({ width, height: 1000 });
  const state = await installApiMocks(page, { role: 'admin' });
  await page.goto('/admin/dashboards');
  await expect(page.getByText('Admin: Dashboards', { exact: true })).toBeVisible();
  await expect(page.getByTitle('Edit (admin)')).toHaveCount(0);
  await expect(page.getByTitle('Lock', { exact: true })).toHaveCount(0);
  await expect(page.getByTitle('Unlock', { exact: true })).toHaveCount(0);
  await page.getByTitle('View', { exact: true }).last().click();
  await expect(page).toHaveURL(new RegExp(`/d/custom-dashboard\\?dashboardId=${customDashboard.id}`));
  await expect(page.getByText('Custom miles', { exact: true })).toBeVisible();
  await expectSavedPlacement(page, width);
  await expectNoLayoutControls(page);
  expect(state.layoutMutations).toEqual([]);
});
}

async function expectNoLayoutControls(page: Page) {
  await expect(page.getByRole('button', { name: /^(Edit dashboard|Save dashboard changes|Discard dashboard changes)$/ })).toHaveCount(0);
  await expect(page.locator('[data-widget-frame="edit"], .react-resizable-handle, [data-widget-edit-control="true"]')).toHaveCount(0);
  await expect(page.getByTitle('Duplicate dashboard')).toHaveCount(0);
  await expect(page.getByTitle('Import from YAML')).toHaveCount(0);
}

async function savedWidgetBoxes(page: Page) {
  return page.locator('[data-dashboard-grid="view"] > [data-widget-id]').evaluateAll(elements =>
    elements.map(element => {
      const { x, y, width, height } = element.getBoundingClientRect();
      return { x, y, width, height };
    }),
  );
}

async function expectSavedPlacement(page: Page, width: number) {
  const grid = page.locator('[data-dashboard-grid="view"]');
  await expect(grid.locator(':scope > [data-widget-id]')).toHaveCount(3);
  const expected = [
    { id: '33333333-3333-3333-3333-333333333333', title: 'Custom miles', column: '1 / span 3', row: '1 / span 2', height: '80px' },
    { id: '33333333-3333-3333-3333-333333333334', title: 'Offset miles', column: '6 / span 5', row: '1 / span 3', height: '120px' },
    { id: '33333333-3333-3333-3333-333333333335', title: 'Lower miles', column: '2 / span 4', row: '5 / span 2', height: '80px' },
  ];
  for (const [index, fixture] of expected.entries()) {
    const frame = grid.locator(':scope > [data-widget-id]').nth(index);
    await expect(frame).toHaveAttribute('data-widget-id', fixture.id);
    await expect(frame).toContainText(fixture.title);
    if (width < 640) {
      expect(await frame.evaluate(element => element.style.minHeight)).toBe(fixture.height);
      await expect(frame).toHaveCSS('min-height', '96px');
      await expect(frame).toHaveCSS('grid-column', 'auto');
      await expect(frame).toHaveCSS('grid-row', 'auto');
    } else {
      await expect(frame).toHaveCSS('grid-column', fixture.column);
      await expect(frame).toHaveCSS('grid-row', fixture.row);
    }
  }
  await expect(grid).toHaveCSS('row-gap', '16px');
  await page.evaluate(async () => {
    await document.fonts.ready;
    await Promise.all(document.getAnimations()
      .filter(animation => animation.effect?.getTiming().iterations !== Infinity)
      .map(animation => animation.finished.catch(() => {})));
  });
  const boxes = await savedWidgetBoxes(page);
  const bounds = (await grid.boundingBox())!;
  if (width < 640) {
    const column = (bounds.width - 16) / 2;
    await expect(grid).toHaveCSS('grid-template-columns', `${column}px ${column}px`);
    for (const [index, box] of boxes.entries()) {
      expect(box.x).toBe(bounds.x + (index % 2) * (column + 16));
      expect(box.width).toBe(column);
      expect(box.y).toBe(index < 2 ? bounds.y : bounds.y + boxes[0]!.height + 16);
      expect(box.height).toBeGreaterThanOrEqual(96);
    }
    expect(boxes[0]!.height).toBe(boxes[1]!.height);
  } else {
    const column = (bounds.width - 11 * 16) / 12;
    const layouts = [{ x: 0, y: 0, w: 3, h: 2 }, { x: 5, y: 0, w: 5, h: 3 }, { x: 1, y: 4, w: 4, h: 2 }];
    for (const [index, layout] of layouts.entries()) {
      const box = boxes[index]!;
      expect(box.x).toBeCloseTo(bounds.x + layout.x * (column + 16), 1);
      expect(box.y).toBe(bounds.y + layout.y * 56);
      expect(box.width).toBeCloseTo(layout.w * column + (layout.w - 1) * 16, 1);
      expect(box.height).toBe(layout.h * 40 + (layout.h - 1) * 16);
    }
  }
}

test.describe('mobile app navigation', () => {
  test.use({ viewport: { width: 390, height: 844 }, hasTouch: true });

  test('uses a bottom, touch-safe navigation bar with Explore access', async ({ page }) => {
    await installApiMocks(page);
    await page.goto('/');

    const navigation = page.getByRole('navigation', { name: 'Primary navigation' });
    await expect(navigation).toBeVisible();
    await expect(navigation).toHaveCSS('position', 'fixed');
    const box = (await navigation.boundingBox())!;
    expect(box.y).toBeGreaterThan(740);
    expect(box.x).toBeGreaterThanOrEqual(0);
    expect(box.x + box.width).toBeLessThanOrEqual(390);
    const home = navigation.getByRole('link', { name: 'Home' });
    const explore = navigation.getByRole('link', { name: 'Explore' });
    const settings = page.getByRole('link', { name: 'Settings', exact: true });
    const signOut = page.getByRole('button', { name: 'Sign out' });

    await expect(home).toHaveAttribute('aria-current', 'page');
    for (const control of [home, explore, settings, signOut]) {
      const box = await control.boundingBox();
      expect(box).not.toBeNull();
      expect(box!.height).toBeGreaterThanOrEqual(44);
    }

    await explore.click();
    await page.getByRole('main').getByRole('link', { name: /Battery/ }).click();
    await expect(page).toHaveURL(/\/battery$/);
    await expect(navigation).toBeVisible();
  });
});

test.describe('mobile dashboard chart viewer', () => {
  test.use({ viewport: { width: 390, height: 844 }, hasTouch: true });

  test('keeps efficiency bars visible and expands into the landscape chart viewer', async ({ page }) => {
    await installApiMocks(page);
    await page.addInitScript(() => {
      Object.defineProperty(HTMLElement.prototype, 'requestFullscreen', {
        configurable: true,
        value: undefined,
      });
    });
    await page.route('**/v1/efficiency/**', async (route) => {
      const path = new URL(route.request().url()).pathname;
      if (path === '/v1/efficiency/trend') {
        return json(route, [
          { ts: '2026-07-01T08:00:00Z', trip_efficiency_wh_mi: 315, rolling_24h_wh_mi: 320 },
          { ts: '2026-07-02T08:00:00Z', trip_efficiency_wh_mi: 305, rolling_24h_wh_mi: 315 },
        ]);
      }
      if (path === '/v1/efficiency/vs-temp') {
        return json(route, [
          { temp_c_low: 5, temp_c_high: 10, avg_efficiency_wh_mi: 360, trip_count: 2, total_miles: 20, avg_speed_mph: 31 },
          { temp_c_low: 15, temp_c_high: 20, avg_efficiency_wh_mi: 310, trip_count: 4, total_miles: 42, avg_speed_mph: 39 },
        ]);
      }
      if (path === '/v1/efficiency/by-mode') {
        return json(route, [
          { drive_mode: 'all_purpose', avg_wh_per_mi: 320, trip_count: 4 },
          { drive_mode: 'sport', avg_wh_per_mi: 370, trip_count: 2 },
        ]);
      }
      return json(route, []);
    });

    await page.goto('/efficiency');
    const chartCatalog = page.locator('[data-widget-id="d3000003-0000-0000-0000-000000000005"]');
    await chartCatalog.scrollIntoViewIfNeeded();
    await chartCatalog.getByRole('button', { name: 'Expand chart' }).click();
    await expect(page.getByText('Rotate for a wider chart')).toBeVisible();

    await page.setViewportSize({ width: 844, height: 390 });
    await expect(page.locator('[data-mobile-chart-viewer="true"]')).toBeVisible();
    await expect.poll(() => page.evaluate(() => ({
      viewerBackground: getComputedStyle(document.querySelector('[data-mobile-chart-viewer="true"]')!).backgroundColor,
      viewerTouchAction: getComputedStyle(document.querySelector('[data-mobile-chart-viewer="true"]')!).touchAction,
      documentOverflow: getComputedStyle(document.documentElement).overflow,
      bodyPosition: getComputedStyle(document.body).position,
      appInert: document.getElementById('root')?.inert,
    }))).toEqual(expect.objectContaining({
      viewerBackground: expect.not.stringMatching(/transparent|rgba\(0,\s*0,\s*0,\s*0\)/),
      viewerTouchAction: 'none',
      documentOverflow: 'hidden',
      bodyPosition: 'fixed',
      appInert: true,
    }));
    const viewer = page.locator('[data-mobile-chart-viewer="true"]');
    const viewerControls = page.locator('[data-mobile-chart-controls="true"]');
    await expect(viewerControls).toHaveAttribute('aria-hidden', 'true');
    await viewer.click({ position: { x: 420, y: 190 } });
    await expect(viewerControls).toHaveAttribute('aria-hidden', 'false');
    await viewer.click({ position: { x: 420, y: 190 } });
    await expect(viewerControls).toHaveAttribute('aria-hidden', 'true');
    await viewer.click({ position: { x: 420, y: 190 } });
    await expect(viewerControls).toHaveAttribute('aria-hidden', 'false');
    await page.getByRole('button', { name: 'Choose chart' }).click();
    await page.getByRole('option', { name: 'Efficiency by Temperature' }).click();

    const allBars = viewer.locator('[data-efficiency-pill-bar="true"]');
    const bars = [];
    for (let index = 0; index < await allBars.count(); index += 1) {
      const bar = allBars.nth(index);
      if (await bar.isVisible()) bars.push(bar);
    }
    expect(bars).toHaveLength(2);
    for (const bar of bars) {
      const box = await bar.boundingBox();
      expect(box?.width).toBeGreaterThan(100);
    }
    await page.getByRole('button', { name: 'Close expanded chart' }).click();
    await expect(page.locator('[data-mobile-chart-viewer="true"]')).toHaveCount(0);
    await expect.poll(() => page.evaluate(() => ({
      documentOverflow: getComputedStyle(document.documentElement).overflow,
      bodyPosition: getComputedStyle(document.body).position,
      appInert: document.getElementById('root')?.inert,
    }))).toEqual(expect.objectContaining({
      bodyPosition: 'static',
      appInert: false,
    }));
  });
});

function loadDashboard(slug: string): DashboardConfig {
  const path = new URL(`../../../../packages/dashboards/src/defaults/${slug}.json`, import.meta.url);
  return JSON.parse(readFileSync(path, 'utf8')) as DashboardConfig;
}

function readJson<T>(path: string): T {
  return JSON.parse(readFileSync(new URL(path, import.meta.url), 'utf8')) as T;
}

async function installApiMocks(page: Page, options: { vehicleStatus?: Record<string, unknown>; role?: string } = {}) {
  const createdDashboards: Array<{ config: DashboardConfig }> = [];
  const layoutMutations: string[] = [];

  await page.addInitScript(() => {
    window.localStorage.setItem('rm-show-dashboard-edit-button:e2e-user', 'true');
  });
  await page.route('**/v2/**', route => {
    const path = new URL(route.request().url()).pathname;
    return json(route, path.endsWith('/preferences/theme')
      ? { schemaVersion: 2, mode: 'dark', selection: { kind: 'builtin', themeId: 'classic' } }
      : { schemaVersion: 2, registryHash: 'test', builtins: [], customThemes: [] });
  });
  await page.routeWebSocket('**/v1/vehicles/live**', (socket) => socket.close());
  await page.route('**/v1/**', async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    const path = url.pathname;
    const method = request.method();
    if (path.includes('/dashboards') && !['GET', 'HEAD'].includes(method)) layoutMutations.push(`${method} ${path}`);

    if (path === '/v1/auth/bootstrap') {
      return json(route, authTokens());
    }
    if (path === '/v1/auth/me') {
      return json(route, {
        user_id: 'e2e-user',
        email: 'editor@panda.test',
        role: options.role ?? 'user',
        default_vehicle_id: 'vehicle-1',
      });
    }
    if (path === '/v1/auth/preferences') {
      return json(route, { units: unitPreferences() });
    }
    if (path === '/v1/vehicles') {
      return json(route, { vehicles: [testVehicle()] });
    }
    if (/^\/v1\/vehicles\/[^/]+\/status$/.test(path)) {
      return json(route, options.vehicleStatus ?? {});
    }
    if (/^\/v1\/vehicles\/[^/]+\/images$/.test(path)) {
      return json(route, { all: [] });
    }
    if (path === '/v1/dashboards' && method === 'GET') {
      return json(route, Array.from(dashboards.values()));
    }
    if (path.startsWith('/v1/dashboards/by-slug/')) {
      const slug = decodeURIComponent(path.slice('/v1/dashboards/by-slug/'.length));
      return json(route, dashboards.get(slug) ?? customDashboard);
    }
    if (method === 'GET' && path.startsWith('/v1/dashboards/')) {
      return json(route, [...dashboards.values()].find(config => config.id === path.split('/').at(-1)) ?? customDashboard);
    }
    if (path === '/v1/charts/effective') {
      const dashboardSlug = url.searchParams.get('dashboard_slug');
      return json(route, chartSeeds
        .filter((chart) => chart.enabled && chart.definition.placements.some(
          (placement) => placement.dashboardSlug === dashboardSlug,
        ))
        .map((chart, index) => ({
          id: `chart-${index + 1}`,
          ownerId: null,
          slug: chart.slug,
          name: chart.name,
          description: chart.description,
          isDefault: true,
          isLocked: false,
          isEnabled: true,
          baselineRevision: 5,
          config: chart.definition,
        })));
    }
    if (path === '/v1/chart-sources') {
      return json(route, chartSources);
    }
    if (path === '/v1/dashboards' && method === 'POST') {
      const body = request.postDataJSON() as { config: DashboardConfig };
      createdDashboards.push(body);
      return json(route, {
        ...body.config,
        id: '44444444-4444-4444-4444-444444444444',
        ownerId: '22222222-2222-2222-2222-222222222222',
        isDefault: false,
        isLocked: false,
      });
    }

    return json(route, fallbackApiResponse(path));
  });

  return { createdDashboards, layoutMutations };
}

function fallbackApiResponse(path: string): unknown {
  if (path === '/v1/metrics/catalog') return { metrics: [] };
  if (path === '/v1/metrics/value') return { value: null, ts: null };
  if (path === '/v1/metrics/series') return [];
  if (path === '/v1/battery/health') return {};
  if (path.startsWith('/v1/battery/')) return [];
  if (path === '/v1/charging/summary' || path === '/v1/charging/chart-series') return {};
  if (path === '/v1/charging') return paginated();
  if (path.startsWith('/v1/charging/')) return [];
  if (path === '/v1/trips/map') return { routes: [] };
  if (path === '/v1/trips') return paginated();
  if (path.startsWith('/v1/trips/')) return [];
  if (path === '/v1/efficiency/summary') {
    return { avg_wh_per_mi: 0, p10_wh_per_mi: 0, p90_wh_per_mi: 0, total_miles: 0 };
  }
  if (path.startsWith('/v1/efficiency/')) return [];
  if (path === '/v1/stats') return {};
  if (path === '/v1/places') return { places: [] };
  return {};
}

function paginated() {
  return { items: [], total: 0, page: 1, per_page: 25, total_pages: 0 };
}

function authTokens() {
  return {
    access_token: 'eyJhbGciOiJub25lIn0.eyJzdWIiOiJlMmUtdXNlciJ9.',
    expires_in: 3600,
    default_vehicle_id: 'vehicle-1',
  };
}

function testVehicle() {
  return {
    id: 'vehicle-1',
    user_id: 'e2e-user',
    rivian_vehicle_id: 'rivian-vehicle-1',
    vin: null,
    model: 'R1T',
    year: 2025,
    trim: 'Adventure',
    color: 'Forest Green',
    battery_capacity_kwh: 135,
    display_name: 'Test R1T',
    created_at: '2026-01-01T00:00:00Z',
    images: null,
    membership_role: 'owner',
  };
}

function unitPreferences() {
  return {
    mode: 'imperial',
    distance_unit: 'miles',
    speed_unit: 'mph',
    temperature_unit: 'fahrenheit',
    pressure_unit: 'psi',
    altitude_unit: 'feet',
    place_radius_unit: 'feet',
    efficiency_display: 'distance_per_energy',
  };
}

function json(route: Route, body: unknown) {
  return route.fulfill({
    status: 200,
    contentType: 'application/json',
    body: JSON.stringify(body),
  });
}
