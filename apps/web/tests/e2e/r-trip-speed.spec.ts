import { expect, test } from '@playwright/test';
import { installRFixture } from './r-experience.fixture';

for (const width of [402, 1280]) {
  test(`GPS-only trip speed remains labelled and readable at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 874 });
    await installRFixture(page, { mode: 'dark', populatedTrip: true, missingTripSpeed: true });
    await page.goto('/trips/trip-1');
    const card = page.locator('[data-trip-speed-profile]');
    await expect(card.getByText('GPS estimated', { exact: true })).toBeVisible();
    await card.scrollIntoViewIfNeeded();
    await expect(card.locator('.recharts-bar-rectangle path').first()).toBeVisible();
    await expect(card).toContainText('100%');
    await expect(card).toContainText('60s apart');
    await expect(card).toContainText('mph');
    await expect(card).toContainText('0-5');

    await page.evaluate(() => {
      const preferences = JSON.parse(localStorage.getItem('rm-unit-preferences')!);
      localStorage.setItem('rm-unit-preferences', JSON.stringify({ ...preferences, mode: 'custom', speed_unit: 'kmh' }));
      window.dispatchEvent(new Event('rm-units-change'));
    });
    await expect(card).toContainText('km/h');
    await expect(card).toContainText('0-8');
    const drive = page.getByRole('heading', { name: 'Drive Chart', exact: true }).locator('../..');
    await expect(drive.getByText('GPS average speed', { exact: true })).toBeVisible();
    await drive.scrollIntoViewIfNeeded();
    const rect = await drive.locator('.u-over').boundingBox();
    expect(rect).not.toBeNull();
    await page.mouse.move(rect!.x + rect!.width / 2, rect!.y + rect!.height / 2);
    await expect(drive).toContainText(/Average \d+:\d+–\d+:\d+/);
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(width);
  });
}
