import { expect, test } from '@playwright/test';
import { installFeatureFixture } from './fullReplacementWebFeatures.fixture';

for (const width of [1280, 390]) {
  for (const role of ['user', 'admin', 'super_user'] as const) {
    test(`fixed settings layouts at ${width}px for ${role}`, async ({ page }) => {
      const fixture = await installFeatureFixture(page, { role });
      await page.addInitScript(() => localStorage.setItem('rm-show-dashboard-edit-button:e2e-user', 'true'));
      await page.setViewportSize({ width, height: 844 });
      await page.goto('/settings?section=dashboards&edit=1');
      await expect(page.getByText('Saved layout', { exact: true })).toBeVisible();
      await expect(page.getByRole('switch', { name: /Show edit button/ })).toHaveCount(0);
      await expect(page.getByRole('button', { name: /^(Edit|Customize|Duplicate|Import|Create dashboard|Reset to default|Restore bundled|Delete)/ })).toHaveCount(0);
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(width);
      const open = page.getByRole('button', { name: /^Open$/ });
      await open.click();
      await expect(page).toHaveURL(/\/d\/saved-layout\?dashboardId=11111111-2222-4333-8444-555555555555$/);
      await expect(page.locator('[data-widget-id="d1000001-0000-0000-0000-000000000006"]')).toHaveCount(1);
      const before = await page.locator('[data-widget-id="d1000001-0000-0000-0000-000000000006"]').boundingBox();
      await page.reload();
      await expect(page.locator('[data-widget-id="d1000001-0000-0000-0000-000000000006"]')).toHaveCount(1);
      const after = await page.locator('[data-widget-id="d1000001-0000-0000-0000-000000000006"]').boundingBox();
      expect(after!.width).toBeCloseTo(before!.width, 2);
      expect(after!.height).toBeCloseTo(before!.height, 2);
      expect(fixture.dashboardRequests).toEqual([]);
      expect(fixture.assignmentRequests).toEqual([]);
    });
  }

  test(`chart definitions cannot change placements through JSON at ${width}px`, async ({ page }) => {
    const fixture = await installFeatureFixture(page);
    await page.setViewportSize({ width, height: 844 });
    await page.goto('/settings?section=charts&edit=1');
    await expect(page.getByRole('button', { name: /Edit dashboard assignments/ })).toHaveCount(0);
    await page.getByRole('button', { name: 'Edit Battery Capacity by Mileage' }).click();
    await page.getByRole('button', { name: 'advanced', exact: true }).click();
    const editor = page.locator('textarea');
    const original = JSON.parse(await editor.inputValue());
    await editor.fill(JSON.stringify({ ...original, placements: [{ dashboardSlug: 'saved-layout' }] }));
    await expect(page.getByText(/Dashboard assignments are fixed/)).toBeVisible();
    await expect(page.getByRole('button', { name: 'Save', exact: true })).toBeDisabled();
    await editor.fill(JSON.stringify(original));
    await expect(page.getByText(/Dashboard assignments are fixed/)).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Save', exact: true })).toBeEnabled();
    expect(fixture.createdCharts).toEqual([]);
    expect(fixture.assignmentRequests).toEqual([]);
    expect(fixture.dashboardRequests).toEqual([]);
  });
}
