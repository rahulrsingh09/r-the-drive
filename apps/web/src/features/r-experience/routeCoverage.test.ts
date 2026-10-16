import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

// New upstream routes require an explicit R coverage decision before integration.
const reviewedRoutes = [
  '/', '/activate', '/admin/dashboards', '/battery', '/battery/phantom-drain',
  '/charging', '/charging/$sessionId', '/connect', '/connect/otp', '/d/$slug',
  '/efficiency', '/explore', '/login', '/settings', '/settings/charts/$chartId',
  '/settings/charts/new', '/settings/themes/$themeId', '/trips', '/trips/$tripId',
  '/users', '/vehicle-health',
];

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
    expect(paths.sort()).toEqual([...reviewedRoutes].sort());
  });
});
