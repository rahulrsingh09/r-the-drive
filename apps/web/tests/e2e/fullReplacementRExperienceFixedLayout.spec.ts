import { expect, test } from '@playwright/test';
import { installGraphFixture } from './r-default-graphs.fixture';
import { installSavedRDashboard, savedRDashboard } from './fullReplacementRExperienceFixedLayout.fixture';

for (const width of [393, 1280]) {
  test(`fullReplacementRExperience saved custom geometry at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    const fixture = await installGraphFixture(page);
    const config = savedRDashboard();
    config.id = '22222222-2222-4222-8222-222222222222';
    config.slug = 'saved-geometry';
    config.isDefault = false;
    config.isLocked = false;
    config.ownerId = '33333333-3333-4333-8333-333333333333';
    const chart = config.widgets.find(widget => widget.options?.chartId === 'charge-level')!;
    chart.layout = { x: 2, y: 1, w: 7, h: 8 };
    config.widgets = [chart];
    const saved = await installSavedRDashboard(page, config);
    await page.goto(`/d/${config.slug}?dashboardId=${config.id}`);
    await page.waitForLoadState('networkidle');
    const card = page.locator(`[data-dashboard-grid] > [data-widget-id="${chart.id}"]`);
    await expect(card).toBeVisible();
    await expect(card.locator('[data-chart-id="charge-level"]')).toBeVisible();
    await expect(page.locator('.rgl-editor')).toHaveCount(0);
    await expect(page.locator('.r-dashboard-surface')).not.toHaveAttribute('data-r-default-layout');
    if (width >= 640) {
      expect(await card.evaluate(element => ({ column: element.style.gridColumn, row: element.style.gridRow })))
        .toEqual({ column: '3 / span 7', row: '2 / span 8' });
    } else {
      expect(await card.evaluate(element => element.style.height)).toBe('320px');
    }
    const before = await card.boundingBox();
    await card.hover();
    await page.mouse.down();
    await page.mouse.move((before?.x ?? 0) + 100, (before?.y ?? 0) + 100);
    await page.mouse.up();
    expect(await card.boundingBox()).toEqual(before);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
    expect([...fixture.writes, ...saved.writes]).toEqual([]);
  });

  for (const role of ['user', 'admin', 'super_user']) {
    test(`fullReplacementRExperience integration rejects edit and admin bypass for ${role} at ${width}px`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ reducedMotion: 'reduce' });
      const fixture = await installGraphFixture(page, { role: 'super_user' });
      await page.route('**/v1/auth/me', route => route.fulfill({
        contentType: 'application/json',
        body: JSON.stringify({ user_id: '33333333-3333-4333-8333-333333333333',
          email: 'owner@example.test', role, default_vehicle_id: 'owner-vehicle' }),
      }));
      await page.addInitScript(() => localStorage.setItem('rm-show-dashboard-edit-button:33333333-3333-4333-8333-333333333333', 'true'));
      for (const path of ['/?edit=1', '/d/dashboard?edit=1', '/admin/dashboards?edit=1', '/settings?section=dashboards']) {
        await page.goto(path);
        await expect(page.locator('.r-page h1')).toBeVisible();
        await page.waitForLoadState('networkidle');
        await expect(page.locator('.rgl-editor, [data-widget-frame="edit"]')).toHaveCount(0);
        await expect(page.getByRole('button', { name: /^(Edit dashboard|Save dashboard changes|Discard dashboard changes|Create dashboard|Import dashboard|Duplicate dashboard)$/ })).toHaveCount(0);
        await expect(page.getByLabel('Import dashboard file')).toHaveCount(0);
        await expect(page.getByRole('link', { name: 'Dashboard library' })).toHaveCount(0);
      }
      expect(fixture.writes.filter(write => write.includes('/dashboards'))).toEqual([]);
    });
  }
}
