import { useQuery, useMutation, useQueryClient, type QueryClient } from '@tanstack/react-query';
import { useAuth, useAuthReady, api, useUpdateDashboardChartFavorite } from '@panda/hooks';
import { DashboardConfigSchema } from './schema';
import { sanitizeDashboardConfig } from './layout';
import type { DashboardConfig, WidgetInstance } from './schema';
import { migrateLegacyWidgetVisibility } from './dashboardVisibility';

const BASE = '/v1/dashboards';
const DASHBOARD_QUERY_STALE_TIME_MS = 60 * 60 * 1000;

interface DashboardRecord {
  id?: string;
  owner_id?: string | null;
  ownerId?: string | null;
  slug?: string;
  name?: string;
  description?: string | null;
  is_default?: boolean;
  isDefault?: boolean;
  is_locked?: boolean;
  isLocked?: boolean;
  config?: unknown;
}

const LEGACY_TRIP_SENSORS = new Map<string, { definitionId: string; valueMode?: string }>(Object.entries({
  miles: { definitionId: 'trip_miles', valueMode: 'sum' },
  count: { definitionId: 'total_trips', valueMode: 'sum' },
  efficiency: { definitionId: 'avg_efficiency' },
  duration: { definitionId: 'avg_trip_duration', valueMode: 'avg' },
}));
const LEGACY_TRIP_METRICS = new Map(
  Array.from(LEGACY_TRIP_SENSORS.values(), sensor => [sensor.definitionId, sensor]),
);

function normalizeWidget(widget: WidgetInstance, charging: boolean): WidgetInstance {
  let result = widget;
  if (widget.componentType === 'custom') {
    if (charging && widget.definitionId === 'charging.network_breakdown') {
      const height = widget.layout.h;
      const h = Number.isFinite(height) ? Math.max(1, Math.min(20, Math.round(height))) : 20;
      if (h !== height) result = { ...widget, layout: { ...widget.layout, h } };
    }
    if (widget.definitionId === 'trips.stat') {
      const options = widget.options ?? {};
      const byMetric = typeof options.metric === 'string' ? LEGACY_TRIP_METRICS.get(options.metric) : undefined;
      const sensor = byMetric ?? (typeof options.stat === 'string' ? LEGACY_TRIP_SENSORS.get(options.stat) : undefined);
      if (sensor) {
        const nextOptions = { ...options };
        delete nextOptions.stat;
        delete nextOptions.metric;
        nextOptions.metric = sensor.definitionId;
        if (sensor.valueMode) nextOptions.valueMode = sensor.valueMode;
        nextOptions.tripSelectionAware = true;
        result = { ...widget, componentType: 'sensor', definitionId: sensor.definitionId, options: nextOptions };
      }
    }
  }
  return migrateLegacyWidgetVisibility(result);
}

export function normalizeDashboardConfig(raw: unknown): DashboardConfig {
  const record = raw as DashboardRecord;
  const config = (record && typeof record === 'object' && record.config && typeof record.config === 'object'
    ? record.config : raw) as Partial<DashboardConfig>;
  const candidate: Record<string, unknown> = { ...config };
  candidate.id = record.id ?? config.id;
  candidate.slug = record.slug ?? config.slug;
  candidate.name = record.name ?? config.name;
  candidate.description = record.description ?? config.description ?? undefined;
  candidate.isDefault = record.isDefault ?? record.is_default ?? config.isDefault;
  candidate.isLocked = record.isLocked ?? record.is_locked ?? config.isLocked;
  candidate.ownerId = record.ownerId ?? record.owner_id ?? config.ownerId ?? null;
  candidate.controls = config.controls ?? { dateRange: true };
  candidate.widgets = Array.isArray(config.widgets) ? config.widgets : [];
  const parsed = DashboardConfigSchema.parse(candidate);
  const charging = parsed.slug === 'charging';
  parsed.widgets = parsed.widgets.map(widget => normalizeWidget(widget, charging));
  return sanitizeDashboardConfig(parsed);
}

function dashboardMutationBody(config: DashboardConfig, admin: boolean) {
  const sanitized = sanitizeDashboardConfig(config);
  return admin
    ? { name: sanitized.name, description: sanitized.description, config: sanitized }
    : { slug: sanitized.slug, name: sanitized.name, description: sanitized.description, config: sanitized };
}

function reconcileDashboardCache(qc: QueryClient, change: DashboardConfig | string) {
  const removing = typeof change === 'string';
  const id = removing ? change : change.id;
  const list = qc.getQueryData<DashboardConfig[]>(['dashboards']);
  const dashboard = removing ? list?.find(entry => entry.id === id) : change;
  const idKey = ['dashboards', 'id', id];
  if (removing) qc.removeQueries({ queryKey: idKey, exact: true });
  else qc.setQueryData<DashboardConfig>(idKey, change);
  if (dashboard) {
    const slugKey = ['dashboards', 'slug', dashboard.slug];
    if (removing) {
      const selected = qc.getQueryData<DashboardConfig>(slugKey);
      if (selected?.id === id) qc.removeQueries({ queryKey: slugKey, exact: true });
    } else {
      qc.setQueryData<DashboardConfig>(slugKey, current => current?.ownerId && !dashboard.ownerId ? current : dashboard);
    }
  }
  qc.setQueryData<DashboardConfig[]>(['dashboards'], current => {
    if (!current) return current;
    const retained: DashboardConfig[] = [];
    for (const entry of current) {
      if (entry.id === id) continue;
      if (!removing && entry.slug === change.slug && entry.ownerId === change.ownerId) continue;
      retained.push(entry);
    }
    if (removing) return retained;
    retained.push(change);
    return retained.sort((left, right) => left.isDefault === right.isDefault
      ? left.name.localeCompare(right.name) : left.isDefault ? -1 : 1);
  });
}

function refreshDashboardCache(qc: QueryClient, dashboard: DashboardConfig, slug: string) {
  reconcileDashboardCache(qc, dashboard);
  for (const queryKey of [['dashboards'], ['dashboards', 'slug', slug]]) {
    void qc.invalidateQueries({ queryKey });
  }
}

const dashboardQueryPolicy = {
  staleTime: DASHBOARD_QUERY_STALE_TIME_MS,
  refetchOnWindowFocus: false,
  refetchOnReconnect: false,
};

async function readDashboard(path: string): Promise<DashboardConfig> {
  const response = await api.apiFetch<unknown>('GET', path);
  return normalizeDashboardConfig(response);
}

function useDashboardLookup(kind: 'slug' | 'id', value: string | null) {
  const ready = useAuthReady();
  const suffix = kind === 'slug' ? `by-slug/${value}` : value;
  return useQuery<DashboardConfig>({
    ...dashboardQueryPolicy,
    queryKey: ['dashboards', kind, value],
    queryFn: () => readDashboard(`${BASE}/${suffix}`),
    enabled: ready && !!value,
    placeholderData: previous => previous,
  });
}

export function useDashboards() {
  const ready = useAuthReady();
  return useQuery<DashboardConfig[]>({
    ...dashboardQueryPolicy,
    queryKey: ['dashboards'],
    enabled: ready,
    queryFn: async () => Array.from(await api.apiFetch<unknown[]>('GET', BASE), normalizeDashboardConfig),
    placeholderData: previous => previous,
  });
}

export function useDashboardBySlug(slug: string | null) {
  return useDashboardLookup('slug', slug);
}

export function useDashboardById(id: string | null) {
  return useDashboardLookup('id', id);
}

type WriteKind = 'create' | 'update' | 'admin';
function useDashboardWrite(kind: WriteKind) {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (config: DashboardConfig) => {
      const body = dashboardMutationBody(config, kind === 'admin');
      const route = kind === 'create' ? BASE : kind === 'admin' ? `/v1/admin/dashboards/${body.config.id}` : `${BASE}/${config.id}`;
      const response = await api.apiFetch<unknown>(kind === 'create' ? 'POST' : 'PUT', route, body);
      return normalizeDashboardConfig(response);
    },
    onSuccess: (data: DashboardConfig, variables: DashboardConfig) => refreshDashboardCache(qc, data, variables.slug),
  });
}

export function useCreateDashboard() { return useDashboardWrite('create'); }
export function useUpdateDashboard() { return useDashboardWrite('update'); }
export function useUpdateAdminDashboard() { return useDashboardWrite('admin'); }

export function useSetAdminDashboardLock() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async ({ id, locked }: { id: string; locked: boolean }) => {
      const response = await api.apiFetch<unknown>('POST', `/v1/admin/dashboards/${id}/lock`, { locked });
      return normalizeDashboardConfig(response);
    },
    onSuccess: data => refreshDashboardCache(qc, data, data.slug),
  });
}

export function useRestoreAdminDashboardDefault() {
  const qc = useQueryClient();
  const userId = useAuth(state => state.userId);
  const updateChartFavorite = useUpdateDashboardChartFavorite();
  return useMutation({
    mutationFn: async (id: string) => {
      if (!DashboardConfigSchema.shape.id.safeParse(id).success) throw new Error('Invalid dashboard ID');
      const response = await api.apiFetch<unknown>('POST', `/v1/admin/dashboards/${id}/restore-default`);
      const restored = normalizeDashboardConfig(response);
      for (const widget of restored.widgets) {
        if (widget.componentType !== 'chart' || widget.managedKey !== 'overview.chart-catalog') continue;
        const chartId = widget.options?.chartId;
        if (typeof chartId === 'string' && chartId) {
          const key = `${restored.slug}:${restored.id}:${widget.id}`;
          try {
            await updateChartFavorite.mutateAsync({ key, chartId });
          } catch {
            // Restore already committed; retain its selection if preference persistence fails.
            qc.setQueryData<{ chart_favorites: Record<string, string> }>(
              ['auth', 'dashboard-chart-favorites', userId],
              current => ({ chart_favorites: Object.assign({}, current?.chart_favorites, { [key]: chartId }) }),
            );
          }
        }
        break;
      }
      return restored;
    },
    onSuccess: data => refreshDashboardCache(qc, data, data.slug),
  });
}

export function useDeleteDashboard() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => api.apiFetch<void>('DELETE', `${BASE}/${id}`),
    onSuccess: (_data, id) => {
      reconcileDashboardCache(qc, id);
      void qc.invalidateQueries({ queryKey: ['dashboards'] });
    },
  });
}

export function useCloneDashboard() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async (id: string) => {
      const response = await api.apiFetch<unknown>('POST', `${BASE}/${id}/clone`);
      return normalizeDashboardConfig(response);
    },
    onSuccess: data => refreshDashboardCache(qc, data, data.slug),
  });
}
