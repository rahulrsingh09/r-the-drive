import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import { rNavigation, rNavigationTab, type RAreaKey } from './RAppLayout';

// New upstream routes require an explicit R coverage decision before integration.
// Each reviewed route also names the tab that is active on it (release 1 mapping).
const reviewedRoutes: Record<string, RAreaKey> = {
  '/': 'now',
  '/trips': 'activity',
  '/trips/$tripId': 'activity',
  '/charging/$sessionId': 'activity',
  '/insights': 'insights',
  '/charging': 'insights',
  '/battery': 'insights',
  '/battery/phantom-drain': 'insights',
  '/efficiency': 'insights',
  '/vehicle-health': 'car',
  '/settings': 'settings',
  '/connect': 'settings',
  '/connect/otp': 'settings',
  '/users': 'settings',
  '/activate': 'car',
  '/login': 'car',
  // Removed in release 2. Each one only redirects; the target page sets the tab.
  '/d/$slug': 'car',
  '/settings/charts/$chartId': 'settings',
  '/settings/charts/new': 'settings',
  '/settings/themes/$themeId': 'settings',
  '/explore': 'car',
  '/admin/dashboards': 'car',
};

const redirects: Record<string, string> = {
  'd.$slug': '/insights',
  'admin.dashboards': '/insights',
  'explore': '/insights',
  'settings.charts.new': '/insights',
  'settings.charts.$chartId': '/insights',
  'settings.themes.$themeId': '/settings',
};

const sample = (path: string) => path.replace(/\$[A-Za-z]+/g, 'sample-id');

describe('R route coverage contract', () => {
  it('accounts for every registered route, including auth and compatibility URLs', () => {
    const root = resolve(process.cwd(), 'src');
    const tree = readFileSync(resolve(root, 'routeTree.ts'), 'utf8');
    const files = [...tree.matchAll(/from '\.\/routes\/([^']+)'/g)]
      .map(match => match[1]).filter(name => name !== '__root');
    const paths = files.map(file => {
      const source = readFileSync(resolve(root, `routes/${file}.tsx`), 'utf8');
      const path = source.match(/path: '([^']+)'/);
      expect(path, `Missing route path in ${file}`).not.toBeNull();
      return path![1];
    });
    expect(paths.sort()).toEqual(Object.keys(reviewedRoutes).sort());
  });

  it.each(Object.entries(redirects))('keeps %s only as a redirect to %s', (file, target) => {
    const source = readFileSync(resolve(process.cwd(), `src/routes/${file}.tsx`), 'utf8');
    expect(source).toContain(`redirect({ to: '${target}', replace: true })`);
    expect(source).not.toMatch(/component:/);
  });
});

describe('R navigation tabs', () => {
  it('shows four tabs in order with their targets', () => {
    expect(rNavigation.map(({ label, to }) => [label, to])).toEqual([
      ['Now', '/'], ['Activity', '/trips'], ['Insights', '/insights'], ['Car', '/vehicle-health'],
    ]);
  });

  it.each(Object.entries(reviewedRoutes))('maps %s to the %s tab', (path, tab) => {
    expect(rNavigationTab(sample(path))).toBe(tab);
  });

  it('maps Settings, connect and detail pages to their areas', () => {
    expect(rNavigationTab('/settings/')).toBe('settings');
    expect(rNavigationTab('/settingsx')).toBe('car');
    expect(rNavigationTab('/trips/')).toBe('activity');
    expect(rNavigationTab('/charging/abc/curve')).toBe('insights');
    expect(rNavigationTab('/battery-other')).toBe('car');
  });
});
