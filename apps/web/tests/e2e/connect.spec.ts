import { expect, test } from '@playwright/test';
import { installRFixture } from './r-experience.fixture';

for (const mode of ['add', 'refresh'] as const) {
  for (const inputMode of ['typed', 'browser-filled-email', 'browser-filled-password'] as const) {
    test(`${mode} Rivian connection submits visible ${inputMode} credentials`, async ({ page }) => {
      await installRFixture(page);
      const credentials = { email: 'driver@example.test', password: ' current-rivian-password &+ ' };
      let submitted: unknown;
      await page.route('**/v1/vehicles/connect', async route => {
        submitted = route.request().postDataJSON();
        await route.fulfill({
          contentType: 'application/json',
          body: JSON.stringify({ requires_otp: true, challenge_id: 'challenge-fixture', vehicles: [] }),
        });
      });
      const search = mode === 'refresh' ? '?mode=refresh&vehicle_id=owner-vehicle' : '';
      await page.goto(`/connect${search}`);
      const email = page.getByLabel('Rivian Email');
      const password = page.getByLabel('Rivian Password');
      if (inputMode === 'typed') {
        await email.fill(credentials.email);
        await password.fill(credentials.password);
      } else {
        await email.fill('previous@example.test');
        await password.fill('previous-fixture-password');
        if (inputMode === 'browser-filled-password') await email.focus();
        await page.evaluate(values => {
          const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;
          setValue.call(document.querySelector('input[type="email"]'), values.email);
          setValue.call(document.querySelector('input[type="password"]'), values.password);
        }, credentials);
      }
      await expect(email).toHaveValue(credentials.email);
      await expect(password).toHaveValue(credentials.password);
      await page.getByRole('button', { name: 'Connect Account', exact: true }).click();
      await expect.poll(() => submitted).toEqual(credentials);
      await expect(page).toHaveURL(/\/connect\/otp\?/);
      const url = new URL(page.url());
      expect(url.searchParams.get('email')).toBe(credentials.email);
      expect(url.searchParams.get('challenge_id')).toBe('challenge-fixture');
      expect(url.searchParams.get('mode')).toBe(mode === 'refresh' ? 'refresh' : null);
      if (mode === 'refresh') {
        expect(url.searchParams.get('vehicle_id')).toBe('owner-vehicle');
      }
    });
  }
}

for (const inputMode of ['typed', 'browser-filled'] as const) {
  test(`Rivian verification submits the visible ${inputMode} code and can retry`, async ({ page }) => {
    await installRFixture(page);
    const submissions: unknown[] = [];
    await page.route('**/v1/vehicles/connect/otp', async route => {
      submissions.push(route.request().postDataJSON());
      await route.fulfill({
        status: 400,
        contentType: 'application/json',
        body: JSON.stringify({ error: { code: 'RIVIAN_OTP_REJECTED', message: 'Fixture rejection' } }),
      });
    });
    await page.goto('/connect/otp?challenge_id=challenge-fixture&email=driver%40example.test');
    const code = page.getByLabel('Verification Code');
    await code.fill('111111');
    if (inputMode === 'typed') {
      await code.fill('654321');
    } else {
      await code.blur();
      await code.evaluate(element => {
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(element, '654321');
      });
    }
    await expect(code).toHaveValue('654321');
    await page.getByRole('button', { name: 'Verify and Connect', exact: true }).click();
    await expect.poll(() => submissions).toEqual([{ challenge_id: 'challenge-fixture', otp_code: '654321' }]);
    await expect(page.getByText('Rivian did not accept that verification code. Check it and try again.').first()).toBeVisible();
    await code.fill('234567');
    await page.getByRole('button', { name: 'Verify and Connect', exact: true }).click();
    await expect.poll(() => submissions.at(-1)).toEqual({ challenge_id: 'challenge-fixture', otp_code: '234567' });
  });
}
