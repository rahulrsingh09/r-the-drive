import { cacheInvalidations, useCacheMutation } from './cacheRuntime';
import { queryPolicies, useResourceQuery } from './queryRuntime';
import type {
  ChartDefinitionV1,
  ChartManagerEntry,
  ChartRecord,
  ChartSourceManifest,
} from '@panda/types';
import { api } from './api';
import { useAuth } from './useAuth';

const CHARTS_BASE = '/v1/charts';
const CHART_SOURCES_BASE = '/v1/chart-sources';
const CHART_QUERY_STALE_TIME_MS = 5 * 60 * 1000;

export interface ChartRecordInput {
  slug: string;
  name: string;
  description?: string | null;
  isEnabled?: boolean;
  config: ChartDefinitionV1;
}

export interface ChartUpdateInput extends ChartRecordInput {
  id: string;
}

export interface ChartPlacementUpdate {
  id: string;
  placements: Array<{ dashboardSlug: string }>;
}

export interface ChartEnabledUpdate {
  id: string;
  isEnabled: boolean;
}

type JsonRecord = Record<string, unknown>;

function recordOf(value: unknown): JsonRecord {
  return value && typeof value === 'object' && !Array.isArray(value) ? value as JsonRecord : {};
}

function normalizeChartRecord(raw: unknown): ChartRecord {
  const row = recordOf(raw);
  const fields = {
    id: ['id', 'id', ''], ownerId: ['ownerId', 'owner_id', null], slug: ['slug', 'slug', ''], name: ['name', 'name', ''],
    isDefault: ['isDefault', 'is_default', false], isLocked: ['isLocked', 'is_locked', false], isEnabled: ['isEnabled', 'is_enabled', true],
    baselineRevision: ['baselineRevision', 'baseline_revision', null],
  } as const;
  const values: Record<string, unknown> = {};
  for (const [key, [camel, snake, fallback]] of Object.entries(fields)) {
    const value = row[camel] ?? row[snake];
    const kind = key === 'baselineRevision' ? 'number' : key.startsWith('is') ? 'boolean' : 'string';
    values[key] = typeof value === kind ? value : fallback;
  }
  if (typeof row.description === 'string') values.description = row.description;
  return { ...values, config: recordOf(row.config) } as unknown as ChartRecord;
}

function normalizeManagerEntry(raw: unknown): ChartManagerEntry {
  const row = recordOf(raw), permissions = recordOf(row.permissions);
  const defaults = { read: true, edit: false, duplicate: true, reset: false, restore: false, delete: false, lock: false };
  const normalizedPermissions = Object.assign({}, defaults, Object.fromEntries(Object.entries(defaults).map(([key, fallback]) => [key, typeof permissions[key] === 'boolean' ? permissions[key] : fallback])));
  const entry: ChartManagerEntry = {
    effective: normalizeChartRecord(row.effective ?? row),
    origin: row.origin === 'override' || row.origin === 'personal' ? row.origin : 'system',
    permissions: normalizedPermissions as ChartManagerEntry['permissions'],
  };
  for (const [property, snake] of [['systemBase', 'system_base'], ['personalOverride', 'personal_override']] as const) {
    const source = row[property] ?? row[snake];
    if (source) entry[property] = normalizeChartRecord(source);
  }
  return entry;
}

function normalizeList<T>(raw: unknown, decode: (value: unknown) => T): T[] {
  const envelope = recordOf(raw);
  const values = [raw, envelope.items, envelope.data].find(Array.isArray) as unknown[] | undefined;
  return values?.map(decode) ?? [];
}

function chartPayload(input: ChartRecordInput) {
  const { slug, name, description = null, isEnabled = true, config } = input;
  return { slug, name, description: description ?? null, isEnabled: isEnabled ?? true, config };
}

const chartCacheKeys = [['charts', 'manager'], ['charts', 'effective'], ['dashboards']] as const;

export function useChartManager() {
  const userId = useAuth((state) => state.userId);
  return useResourceQuery(['charts', 'manager', userId], async () => {
    const response = await api.apiFetch('GET', CHARTS_BASE);
    return normalizeList(response, normalizeManagerEntry);
  }, [], { ...queryPolicies.history, staleTime: CHART_QUERY_STALE_TIME_MS, refetchOnWindowFocus: false });
}

export function useEffectiveCharts(dashboardSlug: string | null) {
  const userId = useAuth((state) => state.userId);
  return useResourceQuery(['charts', 'effective', dashboardSlug, userId], async () => {
    const response = await api.apiFetch('GET', `${CHARTS_BASE}/effective?dashboard_slug=${encodeURIComponent(dashboardSlug ?? '')}`);
    return normalizeList(response, (raw) => normalizeChartRecord(recordOf(raw).effective || raw));
  }, [dashboardSlug], { ...queryPolicies.history, staleTime: CHART_QUERY_STALE_TIME_MS, refetchOnWindowFocus: false });
}

export function useChartSources() {
  return useResourceQuery(['chart-sources'], async () => {
    const response = await api.apiFetch('GET', CHART_SOURCES_BASE);
    return normalizeList(response, (value) => value as ChartSourceManifest);
  }, [], { staleTime: 60 * 60 * 1000, refetchOnWindowFocus: false });
}

function useChartMutation<V>(request: (variables: V) => { method: string; path: string; body?: unknown }) {
  return useCacheMutation((variables: V) => {
    const { method, path, body } = request(variables);
    return api.apiFetch(method, path, body);
  }, () => cacheInvalidations(chartCacheKeys));
}

export function useCreateChart() {
  return useChartMutation((input: ChartRecordInput) => ({ method: 'POST', path: CHARTS_BASE, body: chartPayload(input) }));
}
export function useUpdateChart() {
  return useChartMutation((input: ChartUpdateInput) => ({ method: 'PUT', path: `${CHARTS_BASE}/${input.id}`, body: chartPayload(input) }));
}
export function useCloneChart() {
  return useChartMutation(({ id, slug, name }: { id: string; slug?: string; name?: string }) => ({ method: 'POST', path: `${CHARTS_BASE}/${id}/clone`, body: { slug, name } }));
}
export function useDeleteChart() {
  return useChartMutation((id: string) => ({ method: 'DELETE', path: `${CHARTS_BASE}/${id}` }));
}
export function useResetChart(id?: string) {
  return useChartMutation((overrideId: string = id ?? '') => ({ method: 'DELETE', path: `${CHARTS_BASE}/${overrideId}` }));
}
export function useSetChartEnabled() {
  return useChartMutation(({ id, isEnabled }: ChartEnabledUpdate) => ({ method: 'PATCH', path: `${CHARTS_BASE}/${id}`, body: { isEnabled } }));
}
export function useSetChartPlacements() {
  return useChartMutation(({ id, placements }: ChartPlacementUpdate) => ({ method: 'PATCH', path: `${CHARTS_BASE}/${id}/placements`, body: { placements } }));
}
export function useAdminUpdateChart() {
  return useChartMutation((input: ChartUpdateInput) => ({ method: 'PATCH', path: `/v1/admin/charts/${input.id}`, body: chartPayload(input) }));
}
export function useAdminSetChartLock() {
  return useChartMutation(({ id, locked }: { id: string; locked: boolean }) => ({ method: 'PATCH', path: `/v1/admin/charts/${id}/lock`, body: { locked } }));
}
export function useAdminRestoreChart() {
  return useChartMutation((id: string) => ({ method: 'POST', path: `/v1/admin/charts/${id}/restore` }));
}
