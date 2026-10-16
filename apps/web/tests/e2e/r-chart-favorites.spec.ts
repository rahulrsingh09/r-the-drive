import { expect, test } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { installGraphFixture } from './r-default-graphs.fixture';

for (const width of [393, 1280]) {
  test(`chart favorites start selected and persist independently of pinned graphs ${width}`, async ({ page }) => {
    await page.setViewportSize({ width, height: 852 });
    const fixture = await installGraphFixture(page, { mode: 'dark' });
    await page.route('**/v1/auth/preferences/chart-favorites', async route => {
      if (route.request().method() === 'GET') await new Promise(resolve => setTimeout(resolve, 700));
      await route.fallback();
    });
    const showCatalog = async (slug: string) => {
      const config = JSON.parse(readFileSync(new URL(`../../../../packages/dashboards/src/defaults/${slug}.json`, import.meta.url), 'utf8'));
      const catalog = config.widgets.find((w: { options?: { showPicker?: boolean } }) => w.options?.showPicker === true);
      await page.locator(`[data-widget-id="${catalog.id}"]`).scrollIntoViewIfNeeded();
    };
    for (const slug of ['dashboard', 'battery', 'charging', 'efficiency']) {
      await page.goto(slug === 'dashboard' ? '/' : `/${slug}`);
      await showCatalog(slug);
      const picker = page.getByRole('button', { name: 'Chart', exact: true });
      await picker.scrollIntoViewIfNeeded();
      await picker.click();
      const favorite = page.getByRole('button', { name: / is the default chart$/ });
      await expect(favorite).toHaveCount(1);
      await expect(favorite).toHaveAttribute('aria-pressed', 'true');
      await expect(favorite.locator('svg')).toHaveAttribute('fill', 'currentColor');
      await page.keyboard.press('Escape');
    }
    expect(fixture.writes).toEqual([]);
    await page.goto('/');
    await showCatalog('dashboard');
    const picker = page.getByRole('button', { name: 'Chart', exact: true });
    await picker.scrollIntoViewIfNeeded();
    await picker.click();
    const nextFavorite = page.getByRole('button', { name: /^Set .* as default$/ }).first();
    const title = (await nextFavorite.getAttribute('aria-label'))!.replace(/^Set /, '').replace(/ as default$/, '');
    await nextFavorite.click();
    await expect(page.getByRole('button', { name: `${title} is the default chart` })).toHaveAttribute('aria-pressed', 'true');
    await expect.poll(() => fixture.writes.filter(w => w === 'PUT /v1/auth/preferences/chart-favorites').length).toBe(1);
    await page.reload();
    await showCatalog('dashboard');
    await expect(picker).toContainText(title);
    await picker.click();
    await expect(page.getByRole('button', { name: `${title} is the default chart` })).toHaveAttribute('aria-pressed', 'true');
    await page.keyboard.press('Escape');
    const dashboard = JSON.parse(readFileSync(new URL('../../../../packages/dashboards/src/defaults/dashboard.json', import.meta.url), 'utf8'));
    for (const widget of dashboard.widgets.filter((w: { options?: { showPicker?: boolean } }) => w.options?.showPicker === false)) {
      await page.locator(`[data-widget-id="${widget.id}"]`).scrollIntoViewIfNeeded();
      await expect(page.locator(`[data-widget-id="${widget.id}"] [data-chart-id="${widget.options.chartId}"]`)).toHaveCount(1);
    }
    expect(fixture.writes.filter(w => w.includes('/dashboards/'))).toEqual([]);
  });
}
