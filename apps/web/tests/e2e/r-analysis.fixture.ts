// Synthetic API responses; no live vehicle or account is used.
import { randomUUID } from 'node:crypto';
import type { ChartRecord, ChartDefinitionV1 } from '@panda/types';
import type { DashboardConfig } from '@panda/dashboards';

type Reply = (body: unknown, status?: number) => Promise<unknown>;
type BundledChart = { slug: string; name: string; description: string; enabled: boolean; definition: ChartDefinitionV1 };

export function createAnalysisFixture(dashboards: DashboardConfig[], personal: ChartRecord, bundled: BundledChart[], admin: boolean) {
  const originals = structuredClone(dashboards);
  const charts: ChartRecord[] = [personal, ...bundled.map((chart, index) => ({
    id: `bundled-${index}`, ownerId: null, slug: chart.slug, name: chart.name,
    description: chart.description, isDefault: true, isLocked: true,
    isEnabled: chart.enabled, config: structuredClone(chart.definition),
  }))];
  return async (url: URL, method: string, body: Record<string, unknown>, reply: Reply): Promise<boolean> => {
    const path = url.pathname;
    const done = async (value: unknown, status = 200) => { await reply(value, status); return true; };
    if (path === '/v1/charts/effective') {
      const slug = url.searchParams.get('dashboard_slug');
      return done(charts.filter(chart => chart.isEnabled && chart.config.placements.some(p => p.dashboardSlug === slug || (slug === 'dashboard' && p.dashboardSlug === 'overview'))));
    }
    if (path === '/v1/charts') {
      if (method === 'POST') {
        const chart = { ...body, id: randomUUID(), ownerId: '33333333-3333-4333-8333-333333333333', isDefault: false, isLocked: false } as unknown as ChartRecord;
        charts.push(chart);
        return done(chart);
      }
      return done(charts.map(chart => ({
        effective: chart, ...(chart.isDefault ? { systemBase: chart } : { personalOverride: chart }),
        origin: chart.isDefault ? 'system' : 'personal',
        permissions: { read: true, edit: !chart.isDefault || admin, duplicate: true, delete: !chart.isDefault, reset: false, lock: admin && chart.isDefault, restore: admin && chart.isDefault },
      })));
    }
    const chartMatch = /^\/v1\/(?:admin\/)?charts\/([^/]+)(?:\/([^/]+))?$/.exec(path);
    if (chartMatch) {
      const chart = charts.find(chart => chart.id === chartMatch[1]);
      if (!chart) return done({ error: { message: 'Chart not found' } }, 404);
      const action = chartMatch[2];
      if (action === 'clone') {
        const clone = { ...structuredClone(chart), id: randomUUID(), ownerId: '33333333-3333-4333-8333-333333333333', isDefault: false, isLocked: false, ...body };
        charts.push(clone);
        return done(clone);
      }
      if (action === 'placements') chart.config.placements = body.placements as ChartDefinitionV1['placements'];
      else if (action === 'lock') chart.isLocked = Boolean(body.locked);
      else if (action === 'restore') {
        const source = bundled.find(source => source.slug === chart.slug);
        if (source) chart.config = structuredClone(source.definition);
      } else if (method === 'DELETE') charts.splice(charts.indexOf(chart), 1);
      else if (method !== 'GET') Object.assign(chart, body);
      return done(chart);
    }
    if (path === '/v1/dashboards') {
      if (method === 'POST') {
        const config = { ...(body.config as DashboardConfig), id: randomUUID(), ownerId: '33333333-3333-4333-8333-333333333333', isDefault: false, isLocked: false };
        dashboards.push(config);
        return done(config);
      }
      return done(dashboards);
    }
    if (path.startsWith('/v1/dashboards/by-slug/')) {
      const slug = path.split('/').pop();
      const candidates = dashboards.filter(config => config.slug === slug);
      const config = candidates.find(config => config.ownerId) ?? candidates[0];
      return done(config ?? { error: { message: 'Dashboard not found' } }, config ? 200 : 404);
    }
    const dashboardMatch = /^\/v1\/(?:admin\/)?dashboards\/([^/]+)(?:\/([^/]+))?$/.exec(path);
    if (dashboardMatch) {
      const config = dashboards.find(config => config.id === dashboardMatch[1]);
      if (!config) return done({ error: { message: 'Dashboard not found' } }, 404);
      const action = dashboardMatch[2];
      if (action === 'clone') {
        const clone = { ...structuredClone(config), id: randomUUID(), slug: `${config.slug}-copy`, name: `${config.name} Copy`, ownerId: '33333333-3333-4333-8333-333333333333', isDefault: false, isLocked: false };
        dashboards.push(clone);
        return done(clone);
      }
      if (action === 'lock') config.isLocked = Boolean(body.locked);
      else if (action === 'restore-default') {
        const original = originals.find(item => item.id === config.id);
        if (original) Object.assign(config, structuredClone(original));
      } else if (method === 'DELETE') dashboards.splice(dashboards.indexOf(config), 1);
      else if (method === 'PUT') Object.assign(config, body.config);
      return done(config);
    }
    return false;
  };
}
