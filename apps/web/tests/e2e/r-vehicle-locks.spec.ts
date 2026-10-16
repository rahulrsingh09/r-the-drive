import { expect, test } from '@playwright/test';
import { installGraphFixture } from './r-default-graphs.fixture';

for (const width of [393, 1280]) for (const mode of ['dark', 'light'] as const) {
  test(`one vehicle lock icon updates without opening a modal ${width} ${mode}`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width, height: 852 });
    const fixture = await installGraphFixture(page, { mode });
    const errors: string[] = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.goto('/');
    const icon = page.locator('.r-vehicle-lock');
    await expect(icon).toHaveAccessibleName('Vehicle locked');
    await expect(icon).toHaveCount(1);
    await icon.click();
    await expect(page.getByRole('dialog')).toHaveCount(0);
    await expect.poll(fixture.liveConnections).toBe(1);
    fixture.publishStatus({ doors_locked: false, door_front_left_closed: false, closure_frunk_closed: false });
    await expect(icon).toHaveAccessibleName('Vehicle unlocked');
    const shackle = icon.locator('.r-lock-shackle');
    await expect.poll(() => shackle.evaluate(e => getComputedStyle(e).transform)).not.toBe('none');
    expect(await shackle.evaluate(e => getComputedStyle(e).transitionDuration)).toBe('0.36s');
    await expect(page.locator('.r-door-status, .r-door-sheet, .r-orbit-openings')).toHaveCount(0);
    await expect(page.getByRole('slider', { name: 'Rotate vehicle' })).toBeVisible();
    await page.screenshot({ path: testInfo.outputPath('vehicle-unlocked.png') });
    await page.emulateMedia({ reducedMotion: 'reduce' });
    expect(await shackle.evaluate(e => parseFloat(getComputedStyle(e).transitionDuration))).toBeLessThanOrEqual(0.001);
    fixture.publishStatus({ doors_locked: true });
    await expect(icon).toHaveAccessibleName('Vehicle locked');
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    expect(fixture.writes).toEqual([]);
    expect(errors).toEqual([]);
  });
}

test('unavailable overall lock status is not inferred from individual locks', async ({ page }) => {
  await page.setViewportSize({ width: 320, height: 740 });
  await page.emulateMedia({ reducedMotion: 'reduce' });
  const unavailable = { availability: 'never_seen', ever_seen: false, last_seen_at: null, latest_event_at: null, reason_code: 'invalid_sensor' } as const;
  const fixture = await installGraphFixture(page, { statusOverrides: {
    doors_locked: null, door_front_left_locked: true, door_front_right_locked: true,
  } });
  await page.goto('/');
  const icon = page.locator('.r-vehicle-lock');
  await expect(icon).toHaveAccessibleName('Vehicle lock status unavailable');
  await expect.poll(fixture.liveConnections).toBe(1);
  fixture.publishStatus({ doors_locked: true, field_availability: { doors_locked: unavailable } });
  await expect(icon).toHaveAccessibleName('Vehicle lock status unavailable');
  await expect(icon.locator('.r-lock-shackle')).toHaveCount(0);
  await icon.click();
  await expect(page.getByRole('dialog')).toHaveCount(0);
  expect(fixture.writes).toEqual([]);
});
