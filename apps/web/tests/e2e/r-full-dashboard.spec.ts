import { expect, test } from '@playwright/test';
import { installSavedRDashboard, savedRDashboard } from './fullReplacementRExperienceFixedLayout.fixture';
import { installRFixture } from './r-experience.fixture';

for (const width of [393, 1280]) {
  for (const mode of ['dark', 'light'] as const) {
    test(`complete home dashboard at ${width}px in ${mode}`, async ({ page }, testInfo) => {
      await page.setViewportSize({ width, height: 900 });
      const fixture = await installRFixture(page, { mode, populatedAdmin: true, populatedAnalytics: true, unknownConfiguration: true });
      await page.emulateMedia({ reducedMotion: 'reduce' });
      await page.addInitScript(() => localStorage.setItem('rm-show-dashboard-edit-button:33333333-3333-4333-8333-333333333333', 'true'));
      await page.goto('/d/dashboard');
      await expect(page).toHaveURL(/\/$/);
      await expect(page.getByRole('heading', { name: 'Your Rivian', exact: true })).toBeVisible();
      await expect(page.locator('.r-trip-instruments')).toContainText('Trip distance');
      await expect(page.locator('.r-trip-instruments')).toContainText('1,234');
      await expect(page.locator('.r-trip-instruments')).toContainText('123');
      await expect(page.locator('.r-snapshot-foot')).toContainText('Odometer 12,486');
      await expect(page.getByRole('link', { name: 'Open full dashboard' })).toHaveCount(0);
      const details = page.locator('.r-vehicle-details');
      await expect(details).toContainText('All-Purpose');
      await expect(details).toContainText('2026.35');
      await expect(details).toContainText('68 F');
      await expect(details).toContainText('68%');
      await expect(details).toContainText('218 mi');
      await expect(details).toContainText('80%');
      await expect(page.getByRole('img', { name: 'Vehicle locked', exact: true })).toBeVisible();
      const workspace = page.getByRole('region', { name: 'Dashboard analysis' });
      await workspace.scrollIntoViewIfNeeded();
      await expect(workspace.locator('[data-testid="sensor-chip"]').filter({ hasText: 'Total trip distance' })).toContainText('1,234');
      await workspace.locator('[data-dashboard-grid] > [data-widget-id="d1000001-0000-0000-0000-000000000006"]').scrollIntoViewIfNeeded();
      await page.getByRole('button', { name: 'Chart', exact: true }).click();
      await page.getByRole('option').filter({ hasText: 'Efficiency Trend' }).getByRole('button', { name: 'Efficiency Trend', exact: true }).click();
      await expect(page.getByRole('button', { name: 'Chart', exact: true })).toContainText('Efficiency Trend');
      await expect(workspace.locator('svg').first()).toBeVisible();
      expect(fixture.unsupportedRequests).toEqual([]);
      await expect(page.getByText('The requested resource could not be found.', { exact: true })).toHaveCount(0);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
      await page.evaluate(() => window.scrollTo(0, 0));
      await page.screenshot({ path: testInfo.outputPath('home-dashboard.png'), fullPage: true });
      await page.screenshot({ path: testInfo.outputPath('home-viewport.png') });
      await page.getByRole('button', { name: /Toggle efficiency units/ }).click();
      await expect(workspace.locator('[data-testid="sensor-chip"]').filter({ hasText: 'Avg Efficiency' })).toContainText('320 Wh/mi');
      await page.getByLabel('Dashboard tools', { exact: true }).click();
      await expect(page.getByRole('button', { name: 'Edit dashboard', exact: true })).toHaveCount(0);
      await expect(page.getByRole('link', { name: 'Dashboard library' })).toHaveCount(0);
      await expect(page.getByRole('button', { name: 'Duplicate dashboard', exact: true })).toHaveCount(0);
      await expect(page.getByLabel('Import dashboard file')).toHaveCount(0);
      await expect(workspace).toBeVisible();
    });
  }
}

test('fullReplacementRExperience dashboard tools retain export without layout mutations', async ({ page }) => {
  const fixture = await installRFixture(page, { populatedAdmin: true, populatedAnalytics: true });
  await page.goto('/');
  await page.getByLabel('Dashboard tools', { exact: true }).click();
  const download = page.waitForEvent('download');
  await page.getByRole('button', { name: 'Export dashboard', exact: true }).click();
  expect((await download).suggestedFilename()).toBe('dashboard.yaml');
  await expect(page.getByLabel('Import dashboard file')).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Import dashboard', exact: true })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Duplicate dashboard', exact: true })).toHaveCount(0);
  expect(fixture.writes).toEqual([]);
});

test('software falls back to the selected vehicle health record when the snapshot omits it', async ({ page }) => {
  await installRFixture(page, { populatedAdmin: true, missingSoftware: true });
  await page.goto('/');
  await expect(page.locator('.r-vehicle-details')).toContainText('2026.35');
});

test('fullReplacementRExperience saved custom dashboard renders without replacing Home', async ({ page }) => {
  const fixture = await installRFixture(page, { populatedAdmin: true });
  const config = savedRDashboard();
  config.id = '22222222-2222-4222-8222-222222222222';
  config.slug = 'dashboard-copy';
  config.ownerId = '33333333-3333-4333-8333-333333333333';
  config.isDefault = false;
  config.isLocked = false;
  const saved = await installSavedRDashboard(page, config);
  await page.goto(`/d/dashboard-copy?dashboardId=${config.id}`);
  await expect(page).toHaveURL(/\/d\/dashboard-copy\?dashboardId=/);
  await expect(page.locator('.rgl-editor')).toHaveCount(0);
  await expect(page.locator('.r-dashboard-vehicle')).toBeVisible();
  await expect(page.locator('.r-dashboard-surface')).not.toHaveAttribute('data-r-default-layout');
  await page.getByRole('link', { name: 'Home', exact: true }).click();
  await expect(page).toHaveURL(/\/$/);
  await expect(page.getByRole('region', { name: 'Dashboard analysis' })).toBeVisible();
  expect([...fixture.writes, ...saved.writes]).toEqual([]);
});

for (const width of [393, 1280]) {
  test(`vehicle lock and update availability at ${width}px`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width, height: 900 });
    const statusOverrides = {
      door_front_left_closed: false as boolean | null, door_front_left_locked: false,
      door_rear_right_closed: null as boolean | null, closure_frunk_closed: false,
      doors_locked: false, ota_available_version: '2026.39',
    };
    const fixture = await installRFixture(page, { populatedAdmin: true, statusOverrides });
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await page.goto('/');
    await expect(page.locator('.r-status-glance').filter({ hasText: 'Software' })).toContainText('Update 2026.39 available');
    await expect(page.getByRole('img', { name: 'Vehicle unlocked', exact: true })).toBeVisible();
    await expect(page.getByRole('button', { name: /^Doors & locks:/ })).toHaveCount(0);
    await page.screenshot({ path: testInfo.outputPath('vehicle-lock.png') });
    expect(fixture.writes).toEqual([]);
    statusOverrides.door_front_left_closed = true;
    statusOverrides.door_rear_right_closed = true;
    statusOverrides.closure_frunk_closed = true;
    statusOverrides.doors_locked = true;
    await expect.poll(fixture.liveConnections).toBeGreaterThan(0);
    fixture.publishStatus(statusOverrides);
    await expect(page.getByRole('img', { name: 'Vehicle locked', exact: true })).toBeVisible();
    expect(fixture.writes).toEqual([]);
  });
}

test('fullReplacementRExperience Home honors saved layouts with and without a vehicle widget', async ({ page }) => {
  const fixture = await installRFixture(page, { populatedAdmin: true, role: 'super_user' });
  const config = savedRDashboard();
  const vehicleWidget = config.widgets.find(widget => widget.definitionId === 'overview.vehicle')!;
  config.widgets = config.widgets.filter(widget => widget !== vehicleWidget);
  const saved = await installSavedRDashboard(page, config);
  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'Home', exact: true })).toBeVisible();
  await expect(page.locator('.r-dashboard-vehicle')).toHaveCount(0);
  await expect(page.locator('.rgl-editor')).toHaveCount(0);
  config.widgets.unshift(vehicleWidget);
  await page.reload();
  await expect(page.locator('.r-dashboard-vehicle')).toBeVisible();
  await expect(page.locator('.rgl-editor')).toHaveCount(0);
  expect([...fixture.writes, ...saved.writes].filter(write => write.includes('/dashboards'))).toEqual([]);
});

test('unavailable vehicle readings stay unknown instead of implying a safe state', async ({ page }) => {
  await installRFixture(page, { populatedAdmin: true, statusOverrides: {
    charger_state: null, charger_status: null, doors_locked: null, drive_mode: null,
    door_front_left_closed: null, battery_level: null, range_miles: null, ota_status: null,
  } });
  await page.goto('/');
  await expect(page.locator('.r-energy-summary')).toContainText('Connection unknown');
  await expect(page.locator('.r-energy-summary')).not.toContainText('Not charging');
  await expect(page.locator('.r-soc-summary strong')).toHaveText('—');
  await expect(page.locator('.r-status-glances')).toContainText('Unknown mode');
  await expect(page.locator('.r-status-glances')).toContainText('Update status unknown');
  await expect(page.getByRole('img', { name: 'Vehicle lock status unavailable', exact: true })).toBeVisible();
});
