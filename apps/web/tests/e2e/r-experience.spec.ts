import { expect, test, type Request } from '@playwright/test';
import { installRFixture } from './r-experience.fixture';

const orbitPackUrl = /\/r2-orbit(?:-[^/?]+)?\.bin(?:\?|$)/;

for (const width of [390, 1280]) {
  for (const mode of ['dark', 'light'] as const) {
    test(`R routes remain cohesive at ${width}px in ${mode}`, async ({ page }, testInfo) => {
      test.setTimeout(120_000);
      await page.setViewportSize({ width, height: 900 });
      const errors: string[] = [];
      const requests = new Set<Request>();
      page.on('pageerror', error => errors.push(error.message));
      page.on('request', request => {
        if (/\/v[12]\//.test(request.url())) requests.add(request);
      });
      page.on('requestfinished', request => requests.delete(request));
      page.on('requestfailed', request => requests.delete(request));
      await installRFixture(page, { mode, role: 'super_user', populatedAdmin: true });
      const paths = [
        '/', '/trips', '/charging', '/efficiency', '/battery', '/battery/phantom-drain', '/vehicle-health',
        '/trips/trip-1', '/charging/charge-1', '/explore', '/settings', '/users',
        ...['vehicles','dashboards','charts','units','places','charging','external','api','jobs','raw','backup','account','authentication'].map(section => `/settings?section=${section}`),
        '/d/dashboard', '/d/trips', '/admin/dashboards', '/connect', '/connect/otp?challenge_id=challenge-1',
      ];
      for (const path of paths) {
        await page.goto(path);
        await expect(page.locator('.r-app')).toBeVisible();
        await expect(page.locator('h1').first()).toBeVisible();
        await expect(page.locator('html')).toHaveClass(new RegExp(mode));
        const primary = page.getByRole('navigation', { name: 'Primary navigation' });
        for (const name of ['Home','Trips','Charging','Battery','Explore', ...(width > 900 ? ['Efficiency'] : [])]) {
          await expect(primary.getByRole('link', { name, exact: true })).toBeVisible();
        }
        await expect(page.getByText(/something went wrong/i)).toHaveCount(0);
        expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), path).toBe(true);
        if (['/', '/settings', '/vehicle-health', '/charging'].includes(path)) {
          await page.screenshot({ path: testInfo.outputPath(`${path.replaceAll('/', '') || 'overview'}.png`), fullPage: true });
          // Restoring the viewport can activate deferred charts after capture.
          await page.evaluate(() => new Promise<void>(resolve =>
            requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
        }
        await expect.poll(() => requests.size).toBe(0);
      }
      expect(errors).toEqual([]);
    });
  }
}

test('vehicle rotation loads once on interaction and works with light mode and reduced motion', async ({ page }) => {
  await installRFixture(page, { mode: 'dark' });
  await page.emulateMedia({ reducedMotion: 'reduce' });
  const packs: string[] = [];
  page.on('request', request => { if (request.resourceType() === 'fetch' && orbitPackUrl.test(request.url())) packs.push(request.url()); });
  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'Your Rivian' })).toBeVisible();
  const orbit = page.getByRole('slider', { name: 'Rotate vehicle' });
  await expect(orbit).toBeVisible();
  expect(packs).toHaveLength(0);
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await orbit.focus();
  await page.keyboard.press('Home');
  await expect(orbit).toHaveAttribute('aria-valuenow', '325');
  await expect(page.locator('.r-orbit canvas')).toBeVisible();
  await page.keyboard.press('ArrowRight');
  await expect(orbit).toHaveAttribute('aria-valuenow', '335');
  const box = (await orbit.boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width / 2 + 80, box.y + box.height / 2, { steps: 5 });
  await page.mouse.up();
  await expect(orbit).not.toHaveAttribute('aria-valuenow', '335');
  await page.getByRole('button', { name: 'Switch to light mode' }).click();
  await expect(page.locator('html')).toHaveClass(/light/);
  expect(await page.locator('.r-vehicle-stage').evaluate(element => getComputedStyle(element).backgroundColor))
    .toBe(await page.locator('body').evaluate(element => getComputedStyle(element).backgroundColor));
  await page.getByRole('link', { name: 'Explore', exact: true }).click();
  await page.getByRole('link', { name: 'Home', exact: true }).click();
  await expect(page.locator('.r-orbit canvas')).toBeVisible();
  expect(packs).toHaveLength(1);
});

test('settings navigation, search, units and compatibility URLs have no interface chooser', async ({ page }) => {
  const fixture = await installRFixture(page, { role: 'super_user' });
  await page.goto('/settings');
  await page.getByLabel('Find a setting').fill('time');
  await expect(page.getByRole('button', { name: /Units & time/ })).toBeVisible();
  await expect(page.getByRole('button', { name: /^API Access/ })).toHaveCount(0);
  await page.getByRole('button', { name: /Units & time/ }).click();
  await expect(page).toHaveURL(/section=units/);
  await expect(page.getByLabel('Application time zone')).toBeVisible();
  await page.getByRole('button', { name: 'All settings', exact: true }).click();
  await expect(page.getByLabel('Find a setting')).toBeVisible();
  await page.goto('/settings?section=appearance');
  await expect(page.getByLabel('Application time zone')).toBeVisible();
  await expect(page.getByText('Appearance mode', { exact: true })).toHaveCount(0);
  await page.goto('/settings/themes/old-theme');
  await expect(page).toHaveURL(/\/settings$/);
  await expect(page.getByLabel('Find a setting')).toBeVisible();
  await expect(page.getByRole('radio', { name: /Classic|RAD/ })).toHaveCount(0);
  expect(fixture.writes).toEqual([]);
});

test('changing vehicle clears old readings until the selected vehicle snapshot arrives', async ({ page }) => {
  await installRFixture(page, { secondVehicle: true, statusDelay: 1500 });
  await page.goto('/');
  await expect(page.locator('.r-soc-summary')).toContainText('68%');
  await page.getByLabel('Select vehicle', { exact: true }).click();
  await page.getByRole('option', { name: /Other vehicle/ }).click();
  await expect(page.getByRole('heading', { name: 'Other vehicle' })).toBeVisible();
  await expect(page.locator('.r-soc-summary')).not.toContainText('68%');
  await expect(page.locator('.r-soc-summary')).toContainText('20%');
  await expect(page.getByRole('slider', { name: 'Rotate vehicle' })).toHaveCount(0);
});

for (const width of [390, 1280]) {
  test(`chart editor keeps its focused workflow in the R theme at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    await installRFixture(page, { role: 'super_user', populatedAdmin: true, mode: 'dark' });
    await page.goto('/settings/charts/new');
    await expect(page.locator('.r-editor')).toBeVisible();
    await expect(page.getByRole('heading', { name: 'New chart', exact: true })).toBeVisible();
    await expect(page.getByRole('navigation', { name: 'Chart editor sections' })).toBeVisible();
    for (const section of ['curves', 'display', 'advanced', 'basics']) {
      await page.getByRole('button', { name: section, exact: true }).click();
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true);
    }
    await page.getByRole('button', { name: 'Back to charts' }).click();
    await expect(page.getByRole('dialog')).toBeVisible();
    await page.getByRole('button', { name: /keep editing/i }).click();
    await expect(page.getByRole('heading', { name: 'New chart', exact: true })).toBeVisible();
  });
}

for (const width of [390, 1280]) {
  test(`main trip map supports tap, drag and keyboard inspection at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    const mapErrors: string[] = [];
    page.on('console', message => { if (message.text().includes('maplibre.error')) mapErrors.push(message.text()); });
    await installRFixture(page, { populatedTrip: true });
    await page.goto('/trips/trip-1');
    const canvas = page.getByLabel('Inspect trip route. Tap or drag to select a point; arrow keys move along the route.');
    await expect(canvas).toBeVisible();
    const mapCard = page.getByRole('heading', { name: 'Route Map', exact: true }).locator('../..');
    await canvas.scrollIntoViewIfNeeded();
    await canvas.focus();
    await page.keyboard.press('End');
    await expect(mapCard).toContainText('30:00');
    await page.keyboard.press('Home');
    await expect(mapCard).toContainText('0:00');
    const box = (await canvas.boundingBox())!;
    await page.mouse.click(box.x + box.width * .7, box.y + box.height * .3);
    await expect(mapCard.locator('p')).not.toHaveText('0:00');
    const selected = await mapCard.locator('p').textContent();
    await page.mouse.move(box.x + box.width * .7, box.y + box.height * .3);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width * .3, box.y + box.height * .7, { steps: 12 });
    await page.mouse.up();
    await expect(mapCard.locator('p')).not.toHaveText(selected!);
    await page.getByRole('button', { name: 'Pan map', exact: true }).click();
    await expect(page.getByLabel('Pan trip map', { exact: true })).toBeVisible();
    await page.getByRole('button', { name: 'Inspect route', exact: true }).click();
    await expect(canvas).toBeVisible();
    expect(mapErrors).toEqual([]);
  });
}

test('failed rotation pauses until an explicit retry', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await installRFixture(page);
  let failedLoads = 0;
  await page.route(orbitPackUrl, async route => {
    if (route.request().resourceType() !== 'fetch') return route.fallback();
    failedLoads++;
    return route.fulfill({ status: 503, body: 'Unavailable' });
  });
  await page.goto('/');
  await page.getByRole('button', { name: 'Replay vehicle rotation' }).click();
  await expect(page.getByText('Rotation unavailable. Try again.')).toBeVisible();
  await page.waitForTimeout(700);
  expect(failedLoads).toBe(1);
  await page.getByRole('button', { name: 'Front', exact: true }).click();
  await expect.poll(() => failedLoads).toBe(2);
});

test('cached vehicle readings retain visible browser reconnection status', async ({ page }) => {
  await installRFixture(page);
  await page.routeWebSocket('**/v1/vehicles/live**', socket => socket.close());
  await page.goto('/');
  await expect(page.locator('.r-soc-summary')).toContainText('68%');
  await expect(page.getByText('Reconnecting live updates · Showing last recorded readings.')).toBeVisible();
});
