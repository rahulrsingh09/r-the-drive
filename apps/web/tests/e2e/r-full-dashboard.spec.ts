import { expect, test } from '@playwright/test';
import { readFileSync } from 'node:fs';
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
      await expect(page.getByRole('link', { name: 'Dashboard library' })).toBeVisible();
      await expect(workspace).toBeVisible();
    });
  }
}

test('dashboard tools retain export, safe import and custom copies', async ({ page }) => {
  const fixture = await installRFixture(page, { populatedAdmin: true, populatedAnalytics: true });
  await page.goto('/');
  await page.getByLabel('Dashboard tools', { exact: true }).click();
  const download = page.waitForEvent('download');
  await page.getByRole('button', { name: 'Export dashboard', exact: true }).click();
  expect((await download).suggestedFilename()).toBe('dashboard.yaml');
  const input = page.getByLabel('Import dashboard file');
  await input.setInputFiles({ name: 'invalid.yaml', mimeType: 'text/yaml', buffer: Buffer.from('invalid: dashboard') });
  await expect(page.getByRole('alert')).toContainText('not a valid dashboard');
  expect(fixture.writes).toEqual([]);
  const config = JSON.parse(readFileSync(new URL('../../../../packages/dashboards/src/defaults/dashboard.json', import.meta.url), 'utf8'));
  const file = { name: 'dashboard.yaml', mimeType: 'text/yaml', buffer: Buffer.from(JSON.stringify(config)) };
  await input.setInputFiles(file);
  await expect(page.getByRole('dialog')).toContainText(`${config.widgets.length} widgets`);
  await page.getByRole('button', { name: 'Cancel', exact: true }).click();
  expect(fixture.writes).toEqual([]);
  await input.setInputFiles(file);
  await page.getByRole('button', { name: 'Import copy', exact: true }).click();
  await expect(page).toHaveURL(/\/d\/dashboard-import-.*edit=1.*dashboardId=/);
  await expect(page.locator('.rgl-editor')).toBeVisible();
  expect(fixture.writes).toEqual(['POST /v1/dashboards']);
});

test('software falls back to the selected vehicle health record when the snapshot omits it', async ({ page }) => {
  await installRFixture(page, { populatedAdmin: true, missingSoftware: true });
  await page.goto('/');
  await expect(page.locator('.r-vehicle-details')).toContainText('2026.35');
});

test('duplicating the home dashboard opens an editable custom dashboard without replacing Home', async ({ page }) => {
  await installRFixture(page, { populatedAdmin: true });
  await page.goto('/');
  await page.getByLabel('Dashboard tools', { exact: true }).click();
  await page.getByRole('button', { name: 'Duplicate dashboard', exact: true }).click();
  await expect(page).toHaveURL(/\/d\/dashboard-copy\?edit=1&dashboardId=/);
  await expect(page.locator('.rgl-editor')).toBeVisible();
  await expect(page.locator('.r-dashboard-vehicle')).toBeVisible();
  await page.getByRole('button', { name: 'Discard dashboard changes' }).click();
  await expect(page).toHaveURL(/\/d\/dashboard-copy\?dashboardId=/);
  await expect(page.locator('.r-dashboard-surface')).not.toHaveAttribute('data-r-default-layout');
  await page.getByRole('link', { name: 'Home', exact: true }).click();
  await expect(page).toHaveURL(/\/$/);
  await expect(page.getByRole('region', { name: 'Dashboard analysis' })).toBeVisible();
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

test('Home honors removal, re-adding and saving its vehicle widget', async ({ page }) => {
  const fixture = await installRFixture(page, { populatedAdmin: true, role: 'super_user' });
  await page.goto('/d/dashboard?edit=1');
  const vehicleWidget = page.locator('[data-widget-frame="edit"]').filter({ has: page.locator('[data-widget-definition="overview.vehicle"]') });
  await vehicleWidget.getByRole('button', { name: 'Edit widget settings' }).click();
  await page.getByRole('button', { name: 'Remove component' }).click();
  await page.getByRole('button', { name: 'Delete Widget', exact: true }).click();
  await expect(page.locator('.r-dashboard-vehicle')).toHaveCount(0);
  await page.getByRole('button', { name: 'Save dashboard changes' }).click();
  await expect(page.locator('.rgl-editor')).toHaveCount(0);
  await page.goto('/');
  await expect(page.locator('.r-dashboard-vehicle')).toHaveCount(0);
  await expect(page.getByRole('heading', { name: 'Home', exact: true })).toBeVisible();
  await page.goto('/d/dashboard?edit=1');
  await page.getByPlaceholder('Search widgets…').fill('vehicle');
  await page.getByRole('button', { name: /Vehicle Overview/ }).click();
  await expect(page.locator('.r-dashboard-vehicle')).toBeVisible();
  await page.getByRole('button', { name: 'Save dashboard changes' }).click();
  await expect(page.locator('.rgl-editor')).toHaveCount(0);
  await page.goto('/');
  await expect(page.locator('.r-dashboard-vehicle')).toBeVisible();
  expect(fixture.writes.filter(write => write.includes('/dashboards/'))).toHaveLength(2);
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
