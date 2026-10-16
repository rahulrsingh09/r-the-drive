import { expect, test, type Page } from '@playwright/test';
import { installGraphFixture } from './r-default-graphs.fixture';

async function installParkedFixture(page: Page, mode: 'dark' | 'light') {
  const fixture = await installGraphFixture(page, { mode });
  await page.route(/\/v1\/vehicles\/[^/]+\/(health|parked-energy)(\?|$)/, async route => {
    const health = new URL(route.request().url()).pathname.endsWith('/health');
    const now = new Date().toISOString();
    const body = health ? {
      vehicle_id: 'owner-vehicle', extended_telemetry: { collector: { running: true } },
      vehicle: { name: 'Your Rivian', model: 'R2', trim: 'Performance' },
      latest: { ts: now }, runtime: { worker_health: 'healthy' },
      tires: null, closures: null, software_history: [], thermal_events_30d: 0,
    } : {
      vehicle_id: 'owner-vehicle', generated_at: now, source: 'rivian_reported',
      samples: ['since_parked', '8h', '24h'].map((window, index) => ({
        window, source_at: now, received_at: now, parked_started_at: '2026-10-10T20:00:00Z',
        duration_minutes: [120, 480, 1440][index], total_kwh: [0.35, 0.8, 1.2][index],
        vehicle_systems_kwh: .2, outlets_kwh: .02, climate_kwh: .03, gear_guard_kwh: .1,
        total_range_impact_km: 1.5,
      })),
    };
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });
  });
  return fixture;
}

for (const width of [320, 393, 402, 1280]) for (const mode of ['dark', 'light'] as const) {
  test(`Phantom Drain readable controls and periods ${width} ${mode}`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width, height: 874 });
    await page.emulateMedia({ reducedMotion: 'reduce' });
    const fixture = await installParkedFixture(page, mode);
    await page.goto('/battery/phantom-drain');
    const windows = page.getByRole('group', { name: 'Parked energy window' });
    await expect(windows).toBeVisible();
    for (const label of ['Since parked', '8 hours', '24 hours']) {
      const button = windows.getByRole('button', { name: label, exact: true });
      await button.click();
      await expect(button).toHaveAttribute('aria-pressed', 'true');
      await expect(windows.locator('[aria-pressed="true"]')).toHaveCount(1);
      expect(await button.evaluate(e => e.scrollWidth <= e.clientWidth)).toBe(true);
      const box = (await button.boundingBox())!;
      expect(box.x).toBeGreaterThanOrEqual(0);
      expect(box.x + box.width).toBeLessThanOrEqual(width);
      expect(box.height).toBeGreaterThanOrEqual(44);
    }
    await windows.getByRole('button', { name: 'Since parked' }).click();
    await expect(page.getByTestId('parked-energy-metrics')).toContainText('0.35 kWh');
    const chart = page.locator('[data-chart-renderer="phantom-drain"]');
    await expect(chart.locator('.uplot')).toBeVisible();
    const geometry = await chart.evaluate(e => {
      const plot = e.querySelector('.uplot')!.getBoundingClientRect();
      const card = e.getBoundingClientRect();
      return { right: plot.right, cardRight: card.right, width: plot.width, cardWidth: card.width };
    });
    expect(geometry.right).toBeLessThanOrEqual(geometry.cardRight + 1);
    expect(geometry.width).toBeLessThanOrEqual(geometry.cardWidth);
    const periods = page.getByTestId('phantom-drain-periods-table');
    if (width < 768) {
      await expect(periods.getByRole('table')).toBeHidden();
      await expect(page.getByTestId('phantom-period-card')).toHaveCount(7);
      const card = page.getByTestId('phantom-period-card').first();
      await expect(card).toContainText('0.60%');
      await expect(card).toContainText('90%');
      await card.locator('summary').click();
      await expect(card).toHaveAttribute('open', '');
      await expect(card.getByText('72% → 71%')).toBeVisible();
      await expect(card.getByText('75 W')).toBeVisible();
      await expect(card.getByText('0.25 mi/h')).toBeVisible();
      await card.screenshot({ path: testInfo.outputPath('period-details.png') });
      await periods.getByRole('button', { name: 'Sort rows by' }).click();
      await page.getByRole('option', { name: 'SoC Diff', exact: true }).click();
      await periods.getByRole('button', { name: 'Sort descending' }).click();
      await expect(page.getByTestId('phantom-period-card').first()).toContainText('0.84%');
      await periods.getByRole('searchbox').fill('unmatched');
      await expect(periods.getByRole('heading', { name: 'No matching phantom drain periods' })).toBeVisible();
      await periods.getByRole('searchbox').fill('');
      await expect(page.getByTestId('phantom-period-card')).toHaveCount(7);
      await periods.screenshot({ path: testInfo.outputPath('parked-periods.png') });
    } else {
      await expect(periods.getByRole('table')).toBeVisible();
      await expect(page.getByTestId('mobile-table-cards')).toBeHidden();
    }
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
    await chart.scrollIntoViewIfNeeded();
    await page.screenshot({ path: testInfo.outputPath('chart-viewport.png') });
    await page.evaluate(() => scrollTo(0, 0));
    await page.screenshot({ path: testInfo.outputPath('phantom-full.png'), fullPage: true });
    expect(fixture.writes).toEqual([]);
  });
}

test.describe('phone chart gestures', () => {
  test.use({ viewport: { width: 402, height: 874 }, isMobile: true, hasTouch: true });
  test('tap and horizontal scrub inspect daily drain without blocking vertical scrolling', async ({ page, browserName }, testInfo) => {
    await installParkedFixture(page, 'dark');
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await page.goto('/battery/phantom-drain');
    const chart = page.locator('[data-chart-renderer="phantom-drain"]');
    const plot = chart.locator('.u-over');
    await expect(page.getByTestId('parked-energy-metrics')).toContainText('0.35 kWh');
    await page.getByTestId('phantom-drain-chart-section').scrollIntoViewIfNeeded();
    await expect(plot).toBeVisible();
    const box = (await plot.boundingBox())!;
    await page.touchscreen.tap(box.x + box.width * .3, box.y + box.height * .5);
    const tooltip = chart.getByRole('tooltip');
    await expect(tooltip).toContainText('Daily average rate');
    const first = await tooltip.textContent();
    await page.touchscreen.tap(box.x + box.width * .8, box.y + box.height * .5);
    await expect(tooltip).not.toHaveText(first!);
    const tooltipBox = (await tooltip.boundingBox())!;
    expect(tooltipBox.x + tooltipBox.width).toBeLessThanOrEqual(402);
    expect(await plot.evaluate(e => getComputedStyle(e).touchAction)).toBe('pan-y');
    await page.screenshot({ path: testInfo.outputPath('tap-readout.png') });
    if (browserName === 'chromium') {
      const cdp = await page.context().newCDPSession(page);
      const touch = async (type: 'touchStart' | 'touchMove' | 'touchEnd', x = 0, y = 0) =>
        cdp.send('Input.dispatchTouchEvent', { type, touchPoints: type === 'touchEnd' ? [] : [{ x, y }] });
      await touch('touchStart', box.x + box.width * .8, box.y + box.height * .5);
      for (const fraction of [.7, .6, .5, .4, .3]) await touch('touchMove', box.x + box.width * fraction, box.y + box.height * .5);
      await touch('touchEnd');
      await expect(tooltip).toHaveText(first!);
      const scrollBefore = await page.evaluate(() => scrollY);
      await touch('touchStart', box.x + box.width * .5, box.y + box.height * .7);
      for (const offset of [20, 40, 65, 90, 120]) await touch('touchMove', box.x + box.width * .5, box.y + box.height * .7 - offset);
      await touch('touchEnd');
      await expect.poll(() => page.evaluate(() => scrollY)).toBeGreaterThan(scrollBefore + 30);
      await cdp.detach();
    }
    await expect(chart.getByRole('button', { name: 'Return to full chart view' })).toHaveCount(0);
    await page.getByTestId('phantom-drain-chart-section').getByRole('button', { name: 'Expand chart' }).click();
    await expect(page.getByRole('dialog')).toBeVisible();
    await page.screenshot({ path: testInfo.outputPath('expanded-chart.png') });
  });
});
