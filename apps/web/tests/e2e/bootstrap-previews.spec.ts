import { expect, test } from '@playwright/test';
import { fileURLToPath } from 'node:url';
import { installRFixture } from './r-experience.fixture';

for (const mode of ['dark', 'light'] as const) {
  for (const width of [393, 1280]) {
    test(`repository previews use synthetic data at ${width}px in ${mode}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ reducedMotion: 'reduce' });
      const fixture = await installRFixture(page, {
        mode, populatedAnalytics: true, vehicleName: 'Demo R2',
      });
      for (const [name, route] of [['overview', '/'], ['charging', '/charging']]) {
        await page.goto(route);
        await expect(page.locator('.r-app')).toBeVisible();
        await expect(page.locator('h1').first()).toBeVisible();
        if (route === '/') await expect(page.getByRole('heading', { name: 'Demo R2' })).toBeVisible();
        expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
        if (process.env.PANDA_REFRESH_PREVIEWS === '1') {
          const size = width === 393 ? 'mobile' : 'desktop';
          await page.screenshot({
            path: fileURLToPath(new URL(`../../../../docs/assets/readme/${name}-${size}-${mode}.png`, import.meta.url)),
            fullPage: false,
          });
        }
      }
      expect(fixture.writes).toEqual([]);
      expect(fixture.unsupportedRequests).toEqual([]);
    });
  }
  test(`R documentation mark in ${mode}`, async ({ page }) => {
    await page.setViewportSize({ width: 600, height: 220 });
    await page.setContent(`<html><body style="margin:0"><svg xmlns="http://www.w3.org/2000/svg" width="600" height="220" viewBox="0 0 100 100" role="img" aria-label="R"><path fill="${mode === 'dark' ? '#ffffff' : '#111827'}" d="M22 16h31c19 0 30 10 30 27 0 12-6 21-17 25l20 25H66L48 70H39v23H22V16zm17 16v22h14c9 0 13-4 13-11s-4-11-13-11H39z"/></svg></body></html>`);
    await expect(page.getByRole('img', { name: 'R' })).toBeVisible();
    if (process.env.PANDA_REFRESH_PREVIEWS === '1') {
      await page.screenshot({
        path: fileURLToPath(new URL(`../../../../docs/assets/readme/logo-lockup-${mode}.png`, import.meta.url)),
        omitBackground: true,
      });
    }
  });
}
