import { expect, test } from '@playwright/test';

test.beforeEach(async ({ page }) => {
  await page.routeWebSocket('**/v1/vehicles/live**', (socket) => socket.close());
  await page.route('**/*', async (route) => {
    const url = new URL(route.request().url());
    if (url.hostname !== '127.0.0.1') return route.abort();
    if (!/^\/v[12]\//.test(url.pathname)) return route.continue();
    if (url.pathname === '/v1/auth/bootstrap') {
      return route.fulfill({
        status: 401,
        contentType: 'application/json',
        body: JSON.stringify({ error: { message: 'No fixture session' } }),
      });
    }
    const responses: Record<string, unknown> = {
      '/v1/auth/setup': { setup_required: false },
      '/v1/auth/config': {
        password_login_enabled: true,
        oidc_enabled: false,
        oidc_ready: false,
        oidc_auto_login: false,
      },
    };
    return route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(responses[url.pathname] ?? {}),
    });
  });
});

test('login page renders the auth shell', async ({ page }) => {
  await page.goto('/login');

  await expect(page.locator('.r-auth-brand .r-mark')).toHaveText('R');
  await expect(page.getByText('R:The Drive', { exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Sign in' })).toBeVisible();
  await expect(page.getByLabel('Email')).toBeVisible();
  await expect(page.getByLabel('Password')).toBeVisible();
});

for (const inputMode of ['typed', 'browser-filled-email', 'browser-filled-password'] as const) {
  test(`submits the visible ${inputMode} credentials`, async ({ page }) => {
    const credentials = { email: 'owner@example.test', password: ' current-fixture-password &+ ' };
    let submitted: unknown;
    await page.route('**/v1/auth/login', async route => {
      submitted = route.request().postDataJSON();
      await route.fulfill({
        status: 401,
        contentType: 'application/json',
        body: JSON.stringify({ error: { message: 'Synthetic authentication rejection' } }),
      });
    });
    await page.goto('/login');
    await expect(page.getByLabel('Password')).toBeVisible();
    if (inputMode === 'typed') {
      await page.getByLabel('Email').fill(credentials.email);
      await page.getByLabel('Password').fill(credentials.password);
    } else {
      await page.getByLabel('Email').fill('previous@example.test');
      await page.getByLabel('Password').fill('previous-fixture-password');
      if (inputMode === 'browser-filled-password') await page.getByLabel('Email').focus();
      await page.evaluate(values => {
        const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;
        setValue.call(document.querySelector('input[type="email"]'), values.email);
        setValue.call(document.querySelector('input[type="password"]'), values.password);
      }, credentials);
    }
    await expect(page.getByLabel('Email')).toHaveValue(credentials.email);
    await expect(page.getByLabel('Password')).toHaveValue(credentials.password);
    await page.getByRole('button', { name: 'Sign in', exact: true }).click();
    await expect.poll(() => submitted).toEqual(credentials);
    await expect(page.getByRole('alert')).toContainText('Incorrect email or password');
  });
}
