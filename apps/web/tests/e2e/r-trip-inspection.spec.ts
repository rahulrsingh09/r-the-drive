import { expect, test, type Page } from '@playwright/test';
import { installRFixture } from './r-experience.fixture';

const chartSelector = '[data-chart-renderer="rich-time-series"]';
const mapLabel = 'Inspect trip route. Tap or drag to select a point; arrow keys move along the route.';
const mapTime = (page: Page) => page.getByRole('heading', { name: 'Route Map', exact: true }).locator('..').locator('p');

async function expectSharedTime(page: Page, expected: string) {
  await expect(mapTime(page)).toHaveText(expected);
  await expect.poll(() => page.locator(`${chartSelector} [role="tooltip"]`).allTextContents())
    .toEqual(expect.arrayContaining([
      expect.stringMatching(new RegExp(`^${expected}\\nPower:`)),
      expect.stringMatching(new RegExp(`^${expected}\\nOutside:`)),
      expect.stringMatching(new RegExp(`^${expected}\\nElevation:`)),
      expect.stringMatching(new RegExp(`^${expected}\\nFront Left:`)),
    ]));
  expect(await page.locator(`${chartSelector} .u-cursor-x`).evaluateAll(nodes =>
    nodes.every(node => new DOMMatrix(getComputedStyle(node).transform).m41 >= 0))).toBe(true);
}

for (const width of [393, 1440]) {
  for (const mode of ['light', 'dark'] as const) {
    test(`trip selection links map and graphs and persists at ${width}px in ${mode}`, async ({ page }, testInfo) => {
      await page.setViewportSize({ width, height: 900 });
      const errors: string[] = [];
      page.on('pageerror', error => errors.push(error.message));
      await installRFixture(page, { mode, populatedTrip: true });
      await page.goto('/trips/trip-1');
      const map = page.getByLabel(mapLabel);
      await map.scrollIntoViewIfNeeded();
      await map.focus();
      await page.keyboard.press('End');
      await expectSharedTime(page, '30:00');
      await page.keyboard.press('Home');
      await expectSharedTime(page, '0:00');

      const plot = page.locator(`${chartSelector} .u-over`).first();
      await plot.scrollIntoViewIfNeeded();
      const bounds = (await plot.boundingBox())!;
      await page.mouse.move(bounds.x + bounds.width * .2, bounds.y + bounds.height * .5);
      await page.mouse.down();
      await page.mouse.move(bounds.x + bounds.width * .7, bounds.y + bounds.height * .5, { steps: 12 });
      await page.mouse.up();
      await expectSharedTime(page, '21:00');
      await page.mouse.move(2, 2);
      await page.locator(chartSelector).last().scrollIntoViewIfNeeded();
      await expectSharedTime(page, '21:00');
      await expect(page.getByRole('button', { name: 'Return to full chart view' })).toHaveCount(0);

      await page.locator(chartSelector).last().getByRole('button', { name: 'Front Left', exact: true }).click();
      await page.mouse.move(2, 2);
      await expect(mapTime(page)).toHaveText('21:00');
      await expect(page.locator(chartSelector).last().getByRole('tooltip')).toContainText('21:00');
      await page.locator(chartSelector).last().getByRole('button', { name: 'Front Left', exact: true }).click();
      await expectSharedTime(page, '21:00');

      const layout = await page.locator(chartSelector).evaluateAll(nodes => nodes.map(node => {
        const outer = node.getBoundingClientRect();
        return [...node.querySelectorAll('button, [role="tooltip"]')].every(child => {
          const bounds = child.getBoundingClientRect();
          return bounds.left >= outer.left && bounds.right <= outer.right + 1
            && bounds.top >= outer.top && bounds.bottom <= outer.bottom + 1
            && parseFloat(getComputedStyle(child).fontSize) >= 11;
        });
      }));
      expect(layout).toEqual([true, true, true, true]);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      expect(errors).toEqual([]);
      await page.screenshot({ path: testInfo.outputPath('persistent-trip-inspection.png'), fullPage: true });
    });
  }
}

test.describe('touch trip inspection', () => {
  test.use({ viewport: { width: 393, height: 659 }, hasTouch: true, isMobile: true });

  test('tapping a lower graph updates the map and every other graph', async ({ page }) => {
    await installRFixture(page, { populatedTrip: true });
    await page.goto('/trips/trip-1');
    const plot = page.locator(`${chartSelector} .u-over`).last();
    await plot.scrollIntoViewIfNeeded();
    const bounds = (await plot.boundingBox())!;
    await page.touchscreen.tap(bounds.x + bounds.width * .5, bounds.y + bounds.height * .5);
    await expectSharedTime(page, '15:00');
    await page.getByLabel(mapLabel).scrollIntoViewIfNeeded();
    await expectSharedTime(page, '15:00');
    await expect(plot).toHaveCSS('touch-action', 'pan-y');
  });

  test('horizontal finger drag selects samples while vertical gestures scroll', async ({ page, context, browserName }) => {
    test.skip(browserName !== 'chromium', 'Native touch-drag injection uses Chromium CDP; WebKit covers native tap.');
    await installRFixture(page, { populatedTrip: true });
    await page.goto('/trips/trip-1');
    const plot = page.locator(`${chartSelector} .u-over`).first();
    await plot.scrollIntoViewIfNeeded();
    const bounds = (await plot.boundingBox())!;
    const session = await context.newCDPSession(page);
    const touch = async (type: string, x: number, y: number) => session.send('Input.dispatchTouchEvent', {
      type, touchPoints: type === 'touchEnd' ? [] : [{ x, y, id: 1 }],
    });
    const y = bounds.y + bounds.height * .5;
    await touch('touchStart', bounds.x + bounds.width * .2, y);
    for (let step = 3; step <= 8; step++) {
      await touch('touchMove', bounds.x + bounds.width * step / 10, y);
    }
    await touch('touchEnd', 0, 0);
    await expectSharedTime(page, '24:00');
    const scrollBefore = await page.evaluate(() => scrollY);
    await touch('touchStart', bounds.x + bounds.width * .2, y);
    for (let step = 1; step <= 6; step++) {
      await touch('touchMove', bounds.x + bounds.width * .2, y - step * 20);
    }
    await touch('touchEnd', 0, 0);
    await expect.poll(() => page.evaluate(() => scrollY)).toBeGreaterThan(scrollBefore + 30);
    await expectSharedTime(page, '24:00');
  });
});

test('trip graph readings use the selected metric units at the shared sample', async ({ page }) => {
  await installRFixture(page, { populatedTrip: true, populatedAdmin: true });
  await page.goto('/settings?section=units');
  await page.getByRole('button', { name: /^Metric/ }).click();
  await page.goto('/trips/trip-1');
  const map = page.getByLabel(mapLabel);
  await map.scrollIntoViewIfNeeded();
  await map.focus();
  await page.keyboard.press('End');
  await expectSharedTime(page, '30:00');
  const tooltips = page.locator(`${chartSelector} [role="tooltip"]`);
  await expect(tooltips.nth(0)).toContainText('Recorded speed: 80 km/h');
  await expect(tooltips.nth(1)).toContainText('Outside: 20 °C');
  await expect(tooltips.nth(2)).toContainText('Elevation: 20 m');
  await expect(tooltips.nth(3)).toContainText('Front Left: 138 kPa');
  const histogram = page.getByRole('heading', { name: 'Speed Histogram' }).locator('../..');
  await histogram.scrollIntoViewIfNeeded();
  await expect(histogram.getByText('72-80', { exact: true })).toBeVisible();
  await expect(histogram).toContainText('30m covered · 100%');
  await expect(histogram.getByText('80-89', { exact: true })).toHaveCount(0);
});

for (const width of [393, 1440]) {
  test(`all efficiency variants render populated metric readings at ${width}px`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width, height: 900 });
    await installRFixture(page, { populatedAnalytics: true, populatedAdmin: true, mode: 'dark' });
    await page.goto('/settings?section=units');
    await page.getByRole('button', { name: /^Metric/ }).click();
    await page.goto('/efficiency');
    const widget = page.locator('[data-widget-id="d3000003-0000-0000-0000-000000000005"]');
    await widget.scrollIntoViewIfNeeded();
    const picker = widget.locator('button[aria-haspopup="listbox"]');
    for (const name of ['Efficiency Trend', 'Efficiency by Temperature', 'Efficiency by Drive Mode', 'Efficiency by Tag']) {
      await picker.click();
      await page.getByRole('option').filter({ hasText: name }).click();
      await widget.scrollIntoViewIfNeeded();
      if (name === 'Efficiency Trend') {
        await expect(widget.locator('.u-over')).toBeVisible();
      } else {
        const bars = widget.locator('[data-chart-renderer="efficiency-pill-bars"]');
        const rows = bars.getByRole(width === 393 ? 'button' : 'group');
        await expect(rows.first()).toBeVisible();
        await expect(rows.first()).toHaveAccessibleName(/km\/kWh/);
        if (name === 'Efficiency by Temperature') {
          await expect(rows.first()).toHaveAccessibleName(/129 km, 96.6 km\/h/);
        }
        if (name === 'Efficiency by Tag') {
          await expect(rows.first()).toHaveAccessibleName(/257 km, 100% coverage/);
        }
        const display = await bars.evaluate(element => {
          const probe = document.createElement('span');
          probe.style.backgroundColor = 'var(--rm-chart-accent)';
          element.append(probe);
          const accent = getComputedStyle(probe).backgroundColor;
          probe.remove();
          const filled = [...element.querySelectorAll('[data-efficiency-pill-filled]')]
            .filter(node => node.getBoundingClientRect().width > 0);
          return {
            colorsMatch: filled.every(node => getComputedStyle(node).backgroundColor === accent),
            hasSegments: filled.length > 0,
          };
        });
        expect(display).toEqual({ colorsMatch: true, hasSegments: true });
        if (width === 1440) await expect(bars.locator(':scope > div').first()).toContainText('km/kWh');
      }
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      await widget.screenshot({ path: testInfo.outputPath(`${name.toLowerCase().replaceAll(' ', '-')}.png`) });
    }
  });
}

test.describe('high-density phone chart labels', () => {
  test.use({ viewport: { width: 393, height: 659 }, deviceScaleFactor: 3, isMobile: true, hasTouch: true });

  test('efficiency dates fit their actual CSS-pixel spacing', async ({ page }, testInfo) => {
    await page.addInitScript(() => {
      const labels = new Map<HTMLCanvasElement, Array<{ text: string; x: number; y: number; width: number }>>();
      Object.assign(window, { chartLabels: labels });
      const fillText = CanvasRenderingContext2D.prototype.fillText;
      const clearRect = CanvasRenderingContext2D.prototype.clearRect;
      CanvasRenderingContext2D.prototype.clearRect = function (x, y, w, h) {
        if (x === 0 && y === 0) labels.set(this.canvas, []);
        clearRect.call(this, x, y, w, h);
      };
      CanvasRenderingContext2D.prototype.fillText = function (text, x, y, maxWidth) {
        if (/^Oct \d+$/.test(text)) {
          const current = labels.get(this.canvas) ?? [];
          current.push({ text, x, y, width: this.measureText(text).width });
          labels.set(this.canvas, current);
        }
        if (maxWidth == null) fillText.call(this, text, x, y);
        else fillText.call(this, text, x, y, maxWidth);
      };
    });
    await installRFixture(page, { populatedAnalytics: true, populatedAdmin: true, mode: 'dark' });
    await page.goto('/efficiency');
    const widget = page.locator('[data-widget-id="d3000003-0000-0000-0000-000000000005"]');
    await widget.scrollIntoViewIfNeeded();
    await widget.locator('button[aria-haspopup="listbox"]').click();
    await page.getByRole('option').filter({ hasText: 'Efficiency Trend' }).click();
    await widget.scrollIntoViewIfNeeded();
    const canvas = widget.locator('.uplot canvas');
    await expect(canvas).toBeVisible();
    const read = () => canvas.evaluate(element => {
      const labels = (window as unknown as {
        chartLabels: Map<HTMLCanvasElement, Array<{ text: string; x: number; y: number; width: number }>>;
      }).chartLabels.get(element as HTMLCanvasElement) ?? [];
      return labels.sort((a, b) => a.x - b.x);
    });
    await expect.poll(async () => (await read()).length).toBeGreaterThanOrEqual(2);
    const labels = await read();
    const canvasWidth = await canvas.evaluate(element => (element as HTMLCanvasElement).width);
    expect(labels.length).toBeLessThanOrEqual(3);
    for (const label of labels) {
      expect(label.x - label.width / 2).toBeGreaterThanOrEqual(0);
      expect(label.x + label.width / 2).toBeLessThanOrEqual(canvasWidth);
    }
    for (let i = 1; i < labels.length; i++) {
      expect(labels[i]!.x - labels[i - 1]!.x).toBeGreaterThan((labels[i]!.width + labels[i - 1]!.width) / 2 + 12);
    }
    await widget.screenshot({ path: testInfo.outputPath('iphone-3x-efficiency-dates.png') });
  });
});
