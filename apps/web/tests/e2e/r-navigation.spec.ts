import { expect, test } from '@playwright/test';
import { installRFixture } from './r-experience.fixture';

for (const width of [320, 393, 900, 901, 1280]) for (const mode of ['dark', 'light'] as const) {
  test(`navigation labels and targets have room at ${width}px in ${mode}`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width, height: 900 });
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await installRFixture(page, { mode });
    await page.goto('/');
    const nav = page.getByRole('navigation', { name: 'Primary navigation' });
    const home = nav.getByRole('link', { name: 'Home', exact: true });
    await expect(home).toHaveAttribute('href', '/');
    await expect(home).toHaveAttribute('aria-current', 'page');
    const links = nav.getByRole('link');
    await expect(links).toHaveCount(width <= 900 ? 5 : 6);
    let previousRight = 0;
    for (const link of await links.all()) {
      const box = (await link.boundingBox())!;
      const label = (await link.locator('span').boundingBox())!;
      expect(box.width).toBeGreaterThanOrEqual(44);
      expect(box.height).toBeGreaterThanOrEqual(44);
      expect(box.x).toBeGreaterThanOrEqual(previousRight);
      expect(box.x + box.width).toBeLessThanOrEqual(width);
      expect(label.x - box.x).toBeGreaterThanOrEqual(2);
      expect(box.x + box.width - label.x - label.width).toBeGreaterThanOrEqual(2);
      await expect(link.locator('span')).toBeVisible();
      previousRight = box.x + box.width;
    }
    const navBox = (await nav.boundingBox())!;
    if (width <= 900) {
      expect(navBox.y).toBeGreaterThan(740);
      expect(navBox.y + navBox.height).toBeLessThanOrEqual(900);
    } else {
      const logo = (await page.locator('.r-brand').boundingBox())!;
      const actions = (await page.locator('.r-header-actions').boundingBox())!;
      expect(navBox.y).toBeLessThan(100);
      expect(navBox.x).toBeGreaterThanOrEqual(logo.x + logo.width);
      expect(navBox.x + navBox.width).toBeLessThanOrEqual(actions.x);
    }
    await nav.screenshot({ path: testInfo.outputPath('home-navigation.png') });
    await nav.getByRole('link', { name: 'Battery', exact: true }).click();
    await expect(nav.getByRole('link', { name: 'Battery', exact: true })).toHaveAttribute('aria-current', 'page');
    await expect(home).not.toHaveAttribute('aria-current', 'page');
    await nav.getByRole('link', { name: 'Explore', exact: true }).click();
    await expect(nav.getByRole('link', { name: 'Explore', exact: true })).toHaveAttribute('aria-current', 'page');
    await home.click();
    await expect(page).toHaveURL(/\/$/);
    await expect(home).toHaveAttribute('aria-current', 'page');
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  });
}

for (const width of [393, 1280]) {
  test(`battery section selection is exclusive at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    await installRFixture(page);
    await page.goto('/battery');
    const sections = page.getByRole('navigation', { name: 'Battery sections' });
    const overview = sections.getByRole('link', { name: 'Battery overview', exact: true });
    const drain = sections.getByRole('link', { name: 'Phantom drain', exact: true });
    await expect(sections.locator('[aria-current="page"]')).toHaveCount(1);
    await expect(overview).toHaveAttribute('aria-current', 'page');
    await drain.click();
    await expect(page).toHaveURL(/\/battery\/phantom-drain$/);
    await expect(sections.locator('[aria-current="page"]')).toHaveCount(1);
    await expect(drain).toHaveAttribute('aria-current', 'page');
    await page.reload();
    await expect(sections.locator('[aria-current="page"]')).toHaveCount(1);
    await expect(drain).toHaveAttribute('aria-current', 'page');
    await overview.click();
    await expect(sections.locator('[aria-current="page"]')).toHaveCount(1);
    await expect(overview).toHaveAttribute('aria-current', 'page');
  });
}
