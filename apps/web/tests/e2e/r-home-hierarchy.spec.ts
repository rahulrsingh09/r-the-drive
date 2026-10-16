import { expect, test } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { installRFixture } from './r-experience.fixture';

for (const width of [320, 393, 901, 1280, 1920]) for (const mode of ['dark', 'light'] as const) {
  test(`Home prioritizes the vehicle and recent activity at ${width}px in ${mode}`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width, height: 900 });
    await page.emulateMedia({ reducedMotion: 'reduce' });
    const fixture = await installRFixture(page, { mode, populatedAdmin: true, populatedAnalytics: true, secondVehicle: true });
    await page.goto('/');
    const vehicle = page.locator('.r-home-vehicle');
    const totals = page.locator('.r-home-totals');
    const activity = page.getByRole('complementary', { name: 'Recent activity' });
    const charts = page.getByRole('region', { name: 'Overview charts', exact: true });
    await expect(vehicle.getByRole('heading', { name: 'Your Rivian', exact: true })).toBeVisible();
    await expect(totals.locator('[data-testid="sensor-chip"]')).toHaveCount(4);
    await expect(activity.getByRole('heading', { name: 'Recent trips', exact: true })).toBeVisible();
    const boxes = await Promise.all([vehicle, totals, activity, charts].map(locator => locator.boundingBox()));
    const [car, summary, recent, graph] = boxes.map(box => box!);
    if (width > 900) {
      expect(recent.x).toBeGreaterThan(car.x + car.width);
      expect(Math.abs(recent.y - car.y)).toBeLessThan(12);
      expect(Math.abs(summary.x - recent.x)).toBeLessThan(1);
      expect(Math.abs(summary.width - recent.width)).toBeLessThan(1);
      expect(summary.y).toBeGreaterThan(recent.y + recent.height);
      expect(summary.y - (recent.y + recent.height)).toBeLessThan(48);
      const summaryCards = await totals.locator('[data-ui-card]').evaluateAll(elements =>
        elements.map(element => {
          const { x, y, width, height } = element.getBoundingClientRect();
          const value = element.querySelector('.font-mono')?.getBoundingClientRect();
          const sprite = element.querySelector('[data-testid="sensor-sprite-layer"]')?.getBoundingClientRect();
          return {
            x, y, width, height, fits: element.scrollWidth <= element.clientWidth + 1,
            readable: !value || !sprite || value.bottom + 4 <= sprite.y,
          };
        }));
      expect(summaryCards.every(card => card.fits)).toBe(true);
      expect(summaryCards.every(card => card.readable)).toBe(true);
      expect(Math.abs(summaryCards[0]!.y - summaryCards[1]!.y)).toBeLessThan(1);
      expect(Math.abs(summaryCards[2]!.y - summaryCards[3]!.y)).toBeLessThan(1);
      expect(Math.abs(summaryCards[0]!.x - summaryCards[2]!.x)).toBeLessThan(1);
      expect(summaryCards[2]!.y).toBeGreaterThan(summaryCards[0]!.y + summaryCards[0]!.height);
      expect(graph.y).toBeGreaterThan(Math.max(car.y + car.height, summary.y + summary.height));
    } else {
      expect(summary.y).toBeGreaterThanOrEqual(car.y + car.height);
      expect(recent.y).toBeGreaterThan(summary.y + summary.height);
      expect(graph.y).toBeGreaterThan(recent.y + recent.height);
    }
    const orbit = (await vehicle.locator('.r-orbit').boundingBox())!;
    const trip = (await vehicle.locator('.r-trip-instruments').boundingBox())!;
    const energy = (await vehicle.locator('.r-energy-summary').boundingBox())!;
    expect(trip.y).toBeGreaterThanOrEqual(orbit.y + orbit.height);
    expect(energy.y).toBeGreaterThan(trip.y + trip.height);
    expect(Math.abs(trip.x - energy.x)).toBeLessThan(1);
    expect(Math.abs(trip.width - energy.width)).toBeLessThan(1);
    const socValue = (await vehicle.locator('.r-soc-summary strong').boundingBox())!;
    const rangeValue = (await vehicle.getByRole('link', { name: 'Estimated remaining range' }).locator('strong').boundingBox())!;
    expect(Math.abs(socValue.y - rangeValue.y)).toBeLessThan(2);
    const cards = await page.locator('.r-trip-instruments > a, .r-energy-summary > a, .r-home-totals [data-ui-card]').evaluateAll(elements =>
      elements.map(element => {
        const style = getComputedStyle(element);
        return { background: style.backgroundColor, radius: style.borderRadius };
      }));
    expect(cards).toHaveLength(10);
    expect(new Set(cards.map(card => card.background)).size).toBe(1);
    expect(new Set(cards.map(card => card.radius)).size).toBe(1);
    expect(await page.locator('.r-dashboard-vehicle').evaluate(element => getComputedStyle(element).overflow)).toBe('visible');
    expect(await page.locator('.r-workspace-heading').evaluate(element => element.scrollWidth <= element.clientWidth + 1)).toBe(true);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
    await page.screenshot({ path: testInfo.outputPath('home-viewport.png') });
    await page.screenshot({ path: testInfo.outputPath('home-full.png'), fullPage: true });
    expect(fixture.unsupportedRequests).toEqual([]);
    expect(fixture.writes).toEqual([]);
  });
}

test('a previously shortened vehicle widget cannot create an inner scrollbar on Home', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await installRFixture(page, { populatedAdmin: true });
  const config = JSON.parse(readFileSync(new URL('../../../../packages/dashboards/src/defaults/dashboard.json', import.meta.url), 'utf8'));
  config.widgets[0].layout = { x: 0, y: 0, w: 4, h: 1 };
  await page.route('**/v1/dashboards/by-slug/dashboard', route => route.fulfill({
    contentType: 'application/json', body: JSON.stringify(config),
  }));
  await page.goto('/');
  await expect(page.locator('.r-home-workspace')).not.toHaveAttribute('data-r-default-layout');
  const vehicle = page.locator('.r-dashboard-vehicle');
  await expect(vehicle.getByRole('link', { name: /Software/ })).toBeVisible();
  expect(await vehicle.evaluate(element => element.scrollHeight <= element.clientHeight + 1)).toBe(true);
  expect(await vehicle.evaluate(element => getComputedStyle(element).overflow)).toBe('visible');
  const totalCards = page.locator('.r-home-totals [data-ui-card]');
  await expect(totalCards).toHaveCount(4);
  expect(await totalCards.evaluateAll(elements => elements.every(element => element.scrollHeight <= element.clientHeight + 1))).toBe(true);
});
