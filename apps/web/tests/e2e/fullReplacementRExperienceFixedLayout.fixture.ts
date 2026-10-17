import type { Page } from '@playwright/test';
import type { DashboardConfig } from '@panda/dashboards';
import { readFileSync } from 'node:fs';

export function savedRDashboard() {
  return JSON.parse(readFileSync(new URL('../../../../packages/dashboards/src/defaults/dashboard.json', import.meta.url), 'utf8')) as DashboardConfig;
}

export async function installSavedRDashboard(page: Page, config: DashboardConfig) {
  const writes: string[] = [];
  await page.route(/\/v1\/(?:admin\/)?dashboards(?:\/|$|\?)/, async route => {
    const request = route.request();
    const path = new URL(request.url()).pathname;
    if (request.method() !== 'GET') {
      writes.push(`${request.method()} ${path}`);
      return route.fulfill({ status: 405, contentType: 'application/json', body: '{"error":{"message":"Saved fixture is read-only"}}' });
    }
    const id = path.split('/').pop();
    if (path !== '/v1/dashboards' && id !== config.id && id !== config.slug) return route.fallback();
    return route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(path === '/v1/dashboards' ? [config] : config) });
  });
  return { writes };
}
