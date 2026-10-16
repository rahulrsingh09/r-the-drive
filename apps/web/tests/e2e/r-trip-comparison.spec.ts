import { expect, test } from '@playwright/test';
import { installRFixture } from './r-experience.fixture';

for (const width of [393, 1440]) for (const mode of ['dark', 'light'] as const) {
  test(`multi-trip colors, emphasis and units at ${width}px in ${mode}`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    await installRFixture(page, { mode, populatedAnalytics: true, populatedAdmin: true, tripCount: 20, timezone: 'America/Los_Angeles' });
    await page.addInitScript(() => localStorage.setItem('rm-app-timezone', 'America/Los_Angeles'));
    await page.goto('/trips');
    const table = page.locator('[data-widget-id="d5000005-0000-0000-0000-000000000006"]');
    const key = page.getByRole('group', { name: 'Selected route key' });
    const select = async (index: number) => {
      const label = `Route ${String(index).padStart(2, '0')} · Home`;
      await (width === 393 ? table.getByRole('button') : table.getByRole('row')).filter({ hasText: label }).click();
    };
    for (let index = 1; index <= 10; index++) await select(index);
    await expect(key.getByRole('button')).toHaveCount(10);
    const colors = () => key.getByRole('button').evaluateAll(elements => Object.fromEntries(elements.map(element => [
      element.getAttribute('data-route-id')!, getComputedStyle(element.querySelector('path')!).stroke,
    ])));
    const firstDate = await key.getByRole('button').first().locator('span.block').last().textContent();
    expect(firstDate).toBe('Oct 7, 2026, 11:00 PM');
    const firstTrip = (width === 393 ? table.getByRole('button') : table.getByRole('row')).filter({ hasText: 'Route 01 · Home' });
    await expect(firstTrip).toContainText(firstDate!.trim());
    if (width === 1440) {
      const compact = key.locator('../..');
      await compact.evaluate(element => { (element as HTMLElement).style.height = '304px'; });
      await expect.poll(() => compact.evaluate(element => {
        const map = element.querySelector('.maplibregl-map')!.getBoundingClientRect();
        const key = element.querySelector('[data-trip-route-key]')!.getBoundingClientRect();
        return map.bottom <= key.top + 1 && element.scrollHeight <= element.clientHeight + 1;
      })).toBe(true);
      await compact.evaluate(element => { (element as HTMLElement).style.removeProperty('height'); });
    }
    const before = await colors();
    expect(new Set(Object.values(before)).size).toBe(10);
    await key.getByRole('button').nth(7).click();
    await expect(key.getByRole('button').nth(7)).toHaveAttribute('aria-pressed', 'true');
    await expect(key.getByRole('button')).toHaveCount(10);
    await select(2);
    await expect(key.getByRole('button')).toHaveCount(9);
    for (const [id, color] of Object.entries(await colors())) expect(color).toBe(before[id]);
    await select(2);
    const efficiency = page.locator('[data-testid="sensor-chip"]').filter({ hasText: 'Avg Efficiency' }).first();
    await page.getByRole('button', { name: /Toggle efficiency units/ }).click();
    await expect(efficiency).toContainText('304 Wh/mi');
    await expect((width === 393 ? table.getByRole('button') : table.getByRole('row')).filter({ hasText: 'Route 01 · Home' })).toContainText(/250\s*Wh\/mi/);
    await page.getByRole('button', { name: /Toggle efficiency units/ }).click();
    await expect(efficiency).toContainText('3.3 mi/kWh');
    for (let index = 11; index <= 15; index++) await select(index);
    await table.getByRole('button', { name: 'Next', exact: true }).click();
    for (let index = 16; index <= 20; index++) await select(index);
    await expect(key.getByRole('button')).toHaveCount(20);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  });
}
