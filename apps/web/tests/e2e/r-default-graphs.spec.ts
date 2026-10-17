import { expect, test } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { installGraphFixture } from './r-default-graphs.fixture';
import { installSavedRDashboard, savedRDashboard } from './fullReplacementRExperienceFixedLayout.fixture';

for (const width of [393, 1280]) for (const mode of ['dark', 'light'] as const) {
  test(`default graphs and battery navigation ${width} ${mode}`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width, height: 852 });
    await page.emulateMedia({ reducedMotion: 'reduce' });
    const fixture = await installGraphFixture(page, { mode });
    const errors: string[] = []; page.on('pageerror', e => errors.push(e.message));
    for (const slug of ['dashboard', 'battery', 'charging', 'efficiency']) {
      await page.goto(slug === 'dashboard' ? '/' : `/${slug}`);
      const config = JSON.parse(readFileSync(new URL(`../../../../packages/dashboards/src/defaults/${slug}.json`, import.meta.url), 'utf8'));
      const graphs = config.widgets.filter((w: { options?: { showPicker?: boolean } }) => w.options?.showPicker === false);
      for (const widget of graphs) {
        const card = page.locator(`[data-dashboard-grid] > [data-widget-id="${widget.id}"]`);
        await card.scrollIntoViewIfNeeded();
        await expect(card.locator(`[data-chart-id="${widget.options.chartId}"]`)).toBeVisible();
        await expect(card.getByText(widget.title, { exact: true })).toBeVisible();
        await expect(card.getByRole('button', { name: 'Chart', exact: true })).toHaveCount(0);
        expect(await card.evaluate(e => e.scrollWidth <= e.clientWidth + 1)).toBe(true);
      }
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
      await page.evaluate(() => scrollTo(0, 0));
      await page.screenshot({ path: testInfo.outputPath(`${slug}.png`), fullPage: true });
    }
    await page.getByRole('navigation', { name: 'Primary navigation' }).getByRole('link', { name: 'Battery', exact: true }).click();
    await page.getByRole('navigation', { name: 'Battery sections' }).getByRole('link', { name: 'Phantom drain', exact: true }).click();
    await expect(page).toHaveURL(/\/battery\/phantom-drain$/);
    await expect(page.getByRole('navigation', { name: 'Primary navigation' }).getByRole('link', { name: 'Battery', exact: true })).toHaveAttribute('aria-current', 'page');
    await expect(page.getByText('Avg sleep', { exact: true })).toBeVisible();
    await page.screenshot({ path: testInfo.outputPath('phantom-drain.png'), fullPage: true });
    await page.getByRole('button', { name: 'Last 30 days', exact: true }).click();
    await page.getByRole('button', { name: 'Last 90 days', exact: true }).click();
    await expect(page.getByRole('button', { name: 'Last 90 days', exact: true })).toBeVisible();
    await page.getByRole('navigation', { name: 'Battery sections' }).getByRole('link', { name: 'Battery overview' }).click();
    await expect(page.getByRole('button', { name: 'Last 90 days', exact: true })).toBeVisible();
    expect(errors).toEqual([]);
    expect(fixture.writes.filter(w => w.includes('/dashboards'))).toEqual([]);
  });
}

test('small phone primary targets fit and retain accessible names', async ({ page }) => {
  await page.setViewportSize({ width: 320, height: 740 });
  await installGraphFixture(page);
  await page.goto('/battery');
  const links = page.getByRole('navigation', { name: 'Primary navigation' }).getByRole('link');
  await expect(links).toHaveCount(5);
  for (const link of await links.all()) {
    await expect(link.locator('span')).toBeVisible();
    const box = (await link.boundingBox())!;
    expect(box.width).toBeGreaterThanOrEqual(44);
    expect(box.x).toBeGreaterThanOrEqual(0);
    expect(box.x + box.width).toBeLessThanOrEqual(320);
    expect(await link.evaluate(e => e.scrollWidth <= e.clientWidth)).toBe(true);
  }
  await page.getByRole('link', { name: 'Explore', exact: true }).click();
  await page.getByRole('link', { name: /Phantom drain Parked energy/ }).click();
  await expect(page).toHaveURL(/\/battery\/phantom-drain$/);
});

test('mobile efficiency remains reachable and desktop restores its primary tab', async ({ page }) => {
  await page.setViewportSize({ width: 393, height: 852 });
  await installGraphFixture(page);
  await page.goto('/explore');
  const navigation = page.getByRole('navigation', { name: 'Primary navigation' });
  await expect(navigation.getByRole('link')).toHaveCount(5);
  await page.locator('.r-directory').getByRole('link', { name: /^Efficiency / }).click();
  await expect(page).toHaveURL(/\/efficiency$/);
  await expect(navigation.getByRole('link', { name: 'Explore', exact: true })).toHaveAttribute('aria-current', 'page');
  await page.setViewportSize({ width: 1280, height: 852 });
  await expect(navigation.getByRole('link')).toHaveCount(6);
  await expect(navigation.getByRole('link', { name: 'Efficiency', exact: true })).toHaveAttribute('aria-current', 'page');
});

test('fullReplacementRExperience saved layout keeps an absent pinned graph absent', async ({ page }) => {
  const fixture = await installGraphFixture(page, { role: 'super_user' });
  const config = savedRDashboard();
  config.widgets = config.widgets.filter(widget => widget.id !== 'd1000001-0000-0000-0000-000000000101');
  const saved = await installSavedRDashboard(page, config);
  await page.goto('/');
  await expect(page.getByRole('region', { name: 'Dashboard analysis' })).toBeVisible();
  await expect(page.locator('.rgl-editor')).toHaveCount(0);
  await expect(page.locator('[data-dashboard-grid] > [data-widget-id="d1000001-0000-0000-0000-000000000101"]')).toHaveCount(0);
  await expect(page.locator('[data-dashboard-grid] > [data-widget-id="d1000001-0000-0000-0000-000000000102"]')).toHaveCount(1);
  expect([...fixture.writes, ...saved.writes].filter(w => w.includes('/dashboards/'))).toEqual([]);
});

test('charging, vehicle lock and software states stay explicit', async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 393, height: 852 });
  await installGraphFixture(page, { statusOverrides: { charger_state: 'Charging', charger_status: 'chrgr_sts_connected_charging', time_to_end_of_charge_min: 42, doors_locked: false, door_front_left_closed: false, closure_frunk_closed: false, closure_liftgate_closed: false, ota_available_version: '2026.39' } });
  await page.goto('/');
  const energy = page.locator('.r-energy-summary');
  await expect(energy).toHaveAttribute('data-charging', 'true');
  await expect(energy).toContainText('68%');
  await expect(energy).toContainText('218 mi');
  await expect(energy).toContainText('42m');
  await expect(page.getByRole('img', { name: 'Vehicle unlocked', exact: true })).toBeVisible();
  await expect(page.locator('.r-door-status')).toHaveCount(0);
  await expect(page.locator('.r-status-glances')).toContainText('Update 2026.39 available');
  await expect(page.locator('.r-status-glances')).toContainText('410 ft');
  await energy.scrollIntoViewIfNeeded();
  expect(await page.locator('.r-soc-track > span').evaluate(e => getComputedStyle(e, '::after').animationName)).toBe('r-charge-flow');
  await page.screenshot({ path: testInfo.outputPath('charging-mobile.png') });
  await page.emulateMedia({ reducedMotion: 'reduce' });
  expect(await page.locator('.r-soc-track > span').evaluate(e => getComputedStyle(e, '::after').animationName)).toBe('none');
});
