import * as React from 'react';
import type { BasemapStyleDescriptor, MapStylePreference } from '@panda/types';
import { CHART_COLORS, resolveChartColor } from './ChartProvider';
import { tripRouteColor } from './tripRouteColors';
import { useDocumentPalette } from '../hooks/useDocumentPalette';
import { useDocumentTheme } from '../hooks/useDocumentTheme';
import { isAbortError, reportClientError } from '../lib/clientDiagnostics';
import { useThemeRevision } from '../lib/themeRuntime';
import mapLibreWorkerUrl from 'maplibre-gl/dist/maplibre-gl-worker.mjs?worker&url';
import { installMapInspection } from './mapInspection';

export interface LatLng { lat: number; lng: number; }

export interface TripMapRoute {
  id: string;
  track: LatLng[];
  color?: string;
  colorIndex?: number;
}

export type MapStyleMode = 'dark' | 'light';

/** Resolved by the application data layer before the chart is rendered. */
export interface BasemapConfig {
  enabled: boolean;
  provider_preference?: string;
  resolved_provider?: string;
  revision: string;
  styles?: BasemapStyleDescriptor[];
  attributions?: Array<{ label: string; url?: string | null }>;
  // Legacy fields are retained for consumers upgrading alongside the API.
  carto_api_key_missing?: boolean;
  light_url?: string;
  dark_url?: string;
  attribution?: string | null;
  attribution_url?: string | null;
}

export interface TripMapChartProps {
  track: LatLng[];
  routes?: TripMapRoute[];
  selectedRouteIds?: string[];
  highlightedRouteId?: string | null;
  onRouteClick?: (routeId: string) => void;
  onPointSelect?: (point: LatLng, trackIndex?: number) => void;
  activePoint?: LatLng | null;
  activePointIndex?: number | null;
  startPoint?: LatLng;
  endPoint?: LatLng;
  height?: number;
  className?: string;
  mapStyle?: MapStyleMode;
  mapStylePreference?: MapStylePreference;
  /** Bearer token used only for Panda's same-origin basemap proxy. */
  accessToken?: string | null;
  /** Resolved basemap policy. UI never performs API requests. */
  basemapConfig?: BasemapConfig | null | undefined;
  /** Optional connected-wrapper error copy. */
  basemapError?: string | null;
  /** Retries configuration in the connected wrapper, not in this presentation component. */
  onBasemapRetry?: () => void;
  mapLoader?: typeof loadMapLibre;
}

interface MapSourceApi {
  setData(data: unknown): void;
}

interface MapApi {
  remove(): void;
  on(event: string, cb: (event?: unknown) => void): void;
  on(event: string, layerId: string, cb: (event?: unknown) => void): void;
  off(event: string, cb: (event?: unknown) => void): void;
  resize(): void;
  fitBounds(bounds: [[number, number], [number, number]], options?: { padding?: number; animate?: boolean }): void;
  addSource(id: string, source: unknown): void;
  getSource(id: string): MapSourceApi | undefined;
  removeSource(id: string): void;
  addLayer(layer: unknown): void;
  getLayer(id: string): unknown;
  removeLayer(id: string): void;
  setPaintProperty(layerId: string, name: string, value: unknown): void;
  getCanvas(): { style: CSSStyleDeclaration };
  setStyle(style: unknown): void;
  setPitch?(pitch: number): void;
  setBearing?(bearing: number): void;
  dragRotate?: { enable(): void; disable(): void };
  dragPan?: { enable(): void; disable(): void };
  unproject?: (point: [number, number]) => LatLng;
}

const FALLBACK_ACTIVE_POINT_COLOR = CHART_COLORS.warning;
const ACTIVE_POINT_SOURCE_ID = 'trip-active-point';
const ACTIVE_POINT_LAYER_ID = 'trip-active-point-layer';
const ROUTE_SOURCE_ID = 'trip-routes';
const ROUTE_LAYER_ID = 'trip-routes-line';
const ROUTE_HIT_LAYER_ID = 'trip-routes-hit';
const BASEMAP_PROXY_PLACEHOLDER_ORIGIN = 'https://panda.invalid';

export const NEUTRAL_BASEMAP_CONFIG: BasemapConfig = {
  enabled: false,
  revision: 'neutral',
  dark_url: '',
  light_url: '',
  attribution: null,
  attribution_url: null,
};

function resolveStyleDescriptor(basemap: BasemapConfig, preference: MapStylePreference, mode: MapStyleMode) {
  const index = new Map<string, BasemapStyleDescriptor>();
  for (const descriptor of basemap.styles ?? []) if (!index.has(descriptor.id)) index.set(descriptor.id, descriptor);
  const descriptor = index.get(preference) ?? index.get('follow-theme');
  const field = mode === 'dark' ? 'dark_url' : 'light_url';
  return descriptor ? { descriptor, url: descriptor[field] } : { descriptor: undefined, url: basemap[field] ?? '' };
}


function buildMapLibreStyle(mode: MapStyleMode, basemap: BasemapConfig, preference: MapStylePreference = 'follow-theme') {
  const selection = resolveStyleDescriptor(basemap, preference, mode);
  if (basemap.enabled && selection.descriptor?.kind === 'style' && selection.url) return selection.url;
  const neutral = { version: 8 as const, sources: {}, layers: [{ id: 'neutral-background', type: 'background' as const, paint: { 'background-color': getCssColor('--rm-bg-elevated', CHART_COLORS.muted) } }] };
  if (!basemap.enabled) return neutral;
  const attribution = basemap.attributions ? Array.from(basemap.attributions, item => item.label).join(' | ') : basemap.attribution ?? '';
  return { version: 8 as const,
    sources: { 'carto-base': { type: 'raster' as const, tiles: [selection.url], tileSize: 256, attribution } },
    layers: [{ id: 'background', type: 'raster' as const, source: 'carto-base' }],
  };
}


function basemapSignature(basemap: BasemapConfig, mode: MapStyleMode, preference: MapStylePreference) {
  const resolved = resolveStyleDescriptor(basemap, preference, mode);
  return `${mode}|${preference}|${basemap.enabled}|${basemap.revision}|${resolved.url}|${resolved.descriptor?.kind ?? 'raster'}`;
}

export async function loadMapLibre() {
  const maplibregl = await import('maplibre-gl');
  maplibregl.setWorkerUrl(mapLibreWorkerUrl);
  await import('maplibre-gl/dist/maplibre-gl.css');
  return maplibregl;
}

/**
 * Lazy-loads MapLibre GL to avoid bundling it in SSR/test contexts.
 * The map renders one or more trip polylines.
 */
export function TripMapChart({
  track,
  routes,
  selectedRouteIds = [],
  highlightedRouteId = null,
  onRouteClick,
  onPointSelect,
  activePoint,
  activePointIndex,
  height = 320,
  className,
  mapStyle = 'dark',
  mapStylePreference = 'follow-theme',
  accessToken = null,
  basemapConfig,
  basemapError = null,
  onBasemapRetry,
  mapLoader = loadMapLibre,
}: TripMapChartProps) {
  const containerRef = React.useRef<HTMLDivElement>(null);
  const sessionRef = React.useRef<ReturnType<typeof createMapSession> | null>(null);
  const [inspecting, setInspecting] = React.useState(true);
  const [mapError, setMapError] = React.useState<string | null>(null);
  const [mapRetryVersion, retryInitialization] = React.useReducer((version: number) => version + 1, 0);
  const palette = useDocumentPalette();
  const isDark = useDocumentTheme();
  const themeRevision = useThemeRevision();
  const basemap = basemapConfig ?? NEUTRAL_BASEMAP_CONFIG;
  const routeList = React.useMemo(() => {
    const result: TripMapRoute[] = [];
    for (const [index, route] of (routes?.length ? routes : [{ id: 'trip', track }]).entries()) {
      if (route.track.length > 1) result.push({ ...route, colorIndex: route.colorIndex ?? index });
    }
    return result;
  }, [routes, track]);
  const visibleRoutes = React.useMemo(() => {
    if (!selectedRouteIds.length) return routeList;
    const selected = new Set(selectedRouteIds);
    return routeList.filter(route => selected.has(route.id));
  }, [routeList, selectedRouteIds]);
  const visibleRouteSignature = visibleRoutes.map(route => `${route.id}:${route.track.length}:${serializePoint(route.track[0])}:${serializePoint(route.track.at(-1))}`).join('|');
  const state = React.useRef<MapSessionState>(null!);
  state.current = { track, activePoint, activePointIndex, onPointSelect, onRouteClick, inspecting,
    routes: visibleRoutes, selectedRouteIds, highlightedRouteId, signature: visibleRouteSignature,
    basemap, mode: mapStyle, preference: mapStylePreference, accessToken };

  React.useEffect(() => {
    const container = containerRef.current;
    if (!container || !routeList.length) return;
    let disposed = false;
    let session: ReturnType<typeof createMapSession> | undefined;
    const start = async () => {
      try {
        const library = await mapLoader();
        if (disposed) return;
        const first = routeList[0]!.track[0]!;
        const initial = state.current;
        const map = new library.Map({ container, center: [first.lng, first.lat], zoom: 12,
          style: buildMapLibreStyle(initial.mode, initial.basemap, initial.preference),
          attributionControl: false, maxTileCacheSize: 512,
          transformRequest: url => transformBasemapRequest(url, state.current.accessToken),
        }) as MapApi;
        session = createMapSession(map, () => state.current, setMapError);
        sessionRef.current = session;
      } catch (error) {
        if (disposed) return;
        const current = state.current;
        reportClientError(error, { event: 'maplibre.initialization_failed', area: 'map', operation: 'map-initialize', provider: current.basemap.resolved_provider, style: current.preference, severity: 'error' });
        setMapError('Map tiles unavailable');
      }
    };
    void start();
    return () => { disposed = true; session?.dispose(); sessionRef.current = null; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mapLoader, mapRetryVersion, routeList.length === 0]);

  React.useEffect(() => {
    const container = containerRef.current;
    if (!container || typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(() => sessionRef.current?.resize());
    observer.observe(container);
    return () => observer.disconnect();
  }, []);
  React.useEffect(() => { sessionRef.current?.inspect(inspecting); }, [inspecting]);
  React.useEffect(() => sessionRef.current?.swapStyle(), [basemap, mapStyle, mapStylePreference]);
  React.useEffect(() => { sessionRef.current?.routes(); }, [palette, themeRevision, selectedRouteIds, highlightedRouteId, visibleRouteSignature, visibleRoutes]);
  React.useEffect(() => {
    const frame = requestAnimationFrame(() => sessionRef.current?.activePoint());
    return () => cancelAnimationFrame(frame);
  }, [activePoint, isDark, palette, themeRevision]);
  const retryMap = React.useCallback(() => {
    setMapError(null);
    if (sessionRef.current) sessionRef.current.retry(); else retryInitialization();
  }, []);

  return (
    routeList.length === 0 ? (
      <div
        style={{ height }}
        className={className ?? 'w-full rounded-xl border border-border bg-bg-elevated flex items-center justify-center text-sm text-fg-tertiary'}
      >
        No route points in this period
      </div>
    ) : (
      <div className="relative">
        {onPointSelect && <button type="button" onClick={() => setInspecting(value => !value)}
          className="absolute left-2 top-2 z-10 min-h-10 rounded-lg border border-border bg-bg-surface px-3 text-xs text-fg">
          {inspecting ? 'Pan map' : 'Inspect route'}
        </button>}
        <div
          ref={containerRef}
          style={{ height }}
          className={className ?? 'w-full rounded-xl overflow-hidden'}
        />
        {basemap.enabled && ((basemap.attributions?.length ?? 0) > 0 || basemap.attribution) ? (
          <div className="absolute bottom-1 right-1 max-w-[calc(100%-0.5rem)] rounded bg-bg/80 px-1.5 py-0.5 text-right text-[10px] text-fg-tertiary">
            {basemap.attributions?.map((item, index) => <React.Fragment key={`${item.label}-${index}`}>{index > 0 ? ' · ' : ''}{item.url ? <a href={item.url} target="_blank" rel="noopener noreferrer" className="hover:text-fg">{item.label}</a> : item.label}</React.Fragment>)}
            {!basemap.attributions?.length && (basemap.attribution_url ? <a href={basemap.attribution_url} target="_blank" rel="noopener noreferrer" className="hover:text-fg">{basemap.attribution}</a> : basemap.attribution)}
          </div>
        ) : null}
        {mapError || basemapError ? (
          <div className="absolute inset-0 flex items-center justify-center bg-bg/70 p-4 text-center">
            <div className="rounded-lg border border-border bg-bg-elevated px-3 py-2 text-xs text-fg-secondary shadow-lg">
              <p>{basemapError ?? mapError}</p>
              {(basemapError ? onBasemapRetry : mapError ? retryMap : undefined) ? (
                <button
                  type="button"
                  onClick={() => {
                    if (basemapError) onBasemapRetry?.();
                    else retryMap();
                  }}
                  className="mt-1 font-medium text-accent hover:underline"
                >
                  Retry
                </button>
              ) : null}
            </div>
          </div>
        ) : null}
      </div>
    )
  );
}

interface MapSessionState {
  track: LatLng[];
  routes: TripMapRoute[];
  selectedRouteIds: string[];
  highlightedRouteId: string | null;
  signature: string;
  activePoint?: LatLng | null | undefined;
  activePointIndex?: number | null | undefined;
  onPointSelect?: TripMapChartProps['onPointSelect'];
  onRouteClick?: TripMapChartProps['onRouteClick'];
  inspecting: boolean;
  basemap: BasemapConfig;
  mode: MapStyleMode;
  preference: MapStylePreference;
  accessToken: string | null;
}

function transformBasemapRequest(url: string, token: string | null) {
  try {
    const target = new URL(url, window.location.origin);
    const local = target.origin === window.location.origin;
    const placeholder = target.origin === BASEMAP_PROXY_PLACEHOLDER_ORIGIN;
    if (!target.pathname.startsWith('/v1/external/basemap/') || !(local || placeholder)) return { url };
    const resolved = placeholder ? `${window.location.origin}${target.pathname}${target.search}${target.hash}` : url;
    const request: { url: string; credentials: 'same-origin'; headers?: Record<string, string> } = { url: resolved, credentials: 'same-origin' };
    if (token) request.headers = { Authorization: `Bearer ${token}` };
    return request;
  } catch { return { url }; }
}

function createMapSession(map: MapApi, read: () => MapSessionState, setError: (error: string | null) => void) {
  let ready = false;
  let disposed = false;
  let routeSignature = '';
  let appliedStyle = basemapSignature(read().basemap, read().mode, read().preference);
  let restoreStyle: (() => void) | null = null;
  const lastPoint = { current: null as LatLng | null };
  const inspection = installMapInspection(map, read);
  if (!read().inspecting) inspection.setEnabled(false);
  const synchronizeRoutes = () => {
    const current = read();
    const selected = new Set(current.selectedRouteIds);
    const highlight = current.routes.find(route => route.id === current.highlightedRouteId)?.id ?? null;
    const features = [];
    let west = Infinity, south = Infinity, east = -Infinity, north = -Infinity;
    for (const [index, route] of current.routes.entries()) {
      const coordinates: number[][] = [];
      for (const point of route.track) {
        coordinates.push([point.lng, point.lat]);
        west = Math.min(west, point.lng); south = Math.min(south, point.lat);
        east = Math.max(east, point.lng); north = Math.max(north, point.lat);
      }
      features.push({ type: 'Feature', geometry: { type: 'LineString', coordinates }, properties: {
        id: route.id, color: route.color?.trim() || resolveChartColor(tripRouteColor(route.colorIndex ?? index)),
        selected: selected.has(route.id), emphasized: route.id === highlight,
        opacity: highlight && route.id !== highlight ? 0.18 : 0.95,
      } });
    }
    updateMapSource(map, ROUTE_SOURCE_ID, { type: 'FeatureCollection', features });
    const layers = [
      { id: ROUTE_LAYER_ID, type: 'line', source: ROUTE_SOURCE_ID, paint: {
        'line-color': ['get', 'color'], 'line-width': ['case', ['boolean', ['get', 'emphasized'], false], 7, ['boolean', ['get', 'selected'], false], 5, 3], 'line-opacity': ['get', 'opacity'],
      } },
      { id: ROUTE_HIT_LAYER_ID, type: 'line', source: ROUTE_SOURCE_ID, paint: { 'line-color': 'transparent', 'line-width': 18, 'line-opacity': 0 } },
    ];
    for (const layer of layers) {
      if (map.getLayer(layer.id)) continue;
      map.addLayer(layer);
      if (layer.id !== ROUTE_HIT_LAYER_ID) continue;
      map.on('click', layer.id, event => {
        const id = (event as { features?: Array<{ properties?: { id?: unknown } }> } | undefined)?.features?.[0]?.properties?.id;
        if (typeof id === 'string') read().onRouteClick?.(id);
      });
      map.on('mouseenter', layer.id, () => { map.getCanvas().style.cursor = read().onRouteClick ? 'pointer' : ''; });
      map.on('mouseleave', layer.id, () => { map.getCanvas().style.cursor = ''; });
    }
    if (current.routes.length && routeSignature !== current.signature) {
      map.fitBounds([[west, south], [east, north]], { padding: 48, animate: false });
      map.resize();
    }
    routeSignature = current.signature;
  };
  const restore = () => {
    if (disposed) return;
    ready = true;
    routeSignature = '';
    lastPoint.current = null;
    setError(null);
    synchronizeRoutes();
    syncActivePoint(map, read().activePoint, lastPoint);
    applyPerspective(map, read().basemap, read().preference);
  };
  const replaceStyle = (oneShot = false) => {
    if (restoreStyle) map.off('style.load', restoreStyle);
    const current = read();
    appliedStyle = basemapSignature(current.basemap, current.mode, current.preference);
    restoreStyle = () => {
      if (oneShot && restoreStyle) { map.off('style.load', restoreStyle); restoreStyle = null; }
      restore();
    };
    map.on('style.load', restoreStyle);
    map.setStyle(buildMapLibreStyle(current.mode, current.basemap, current.preference));
  };
  const load = () => {
    restore();
    const current = read();
    if (appliedStyle !== basemapSignature(current.basemap, current.mode, current.preference)) replaceStyle(true);
    requestAnimationFrame(() => { if (!disposed) map.resize(); });
  };
  const error = (event?: unknown) => {
    const details = mapLibreErrorDetails(event);
    if (isAbortError(details.error)) return;
    const current = read();
    reportClientError(details.error, { event: 'maplibre.error', area: 'map', operation: 'map-resource-load', status: details.status, provider: current.basemap.resolved_provider, style: current.preference, resourceKind: details.resourceKind, sourceId: details.sourceId, url: details.url, severity: 'error' });
    if (current.basemap.enabled) setError('Map tiles unavailable');
  };
  map.on('load', load);
  map.on('error', error);
  return {
    resize: () => map.resize(),
    inspect: inspection.setEnabled,
    routes() {
      if (!ready) return;
      if (map.getLayer('neutral-background')) map.setPaintProperty('neutral-background', 'background-color', getCssColor('--rm-bg-elevated', CHART_COLORS.muted));
      synchronizeRoutes();
    },
    activePoint() { if (ready) syncActivePoint(map, read().activePoint, lastPoint); },
    swapStyle() {
      if (!ready) return;
      replaceStyle();
      return () => { if (restoreStyle) map.off('style.load', restoreStyle); restoreStyle = null; };
    },
    retry() { appliedStyle = ''; map.setStyle(buildMapLibreStyle(read().mode, read().basemap, read().preference)); },
    dispose() {
      disposed = true;
      inspection();
      if (restoreStyle) map.off('style.load', restoreStyle);
      map.off('load', load); map.off('error', error);
      map.remove();
    },
  };
}

function updateMapSource(map: MapApi, id: string, data: unknown) {
  const existing = map.getSource(id);
  if (existing) existing.setData(data); else map.addSource(id, { type: 'geojson', data });
}

function mapLibreErrorDetails(event: unknown): {
  error: unknown;
  status?: number | undefined;
  resourceKind?: string | undefined;
  sourceId?: string | undefined;
  url?: string | undefined;
} {
  if (!isRecord(event)) return { error: event };

  const nested = isRecord(event.error) ? event.error : {};
  const fields = {
    url: [event.url, nested.url, event.resource, event.source],
    sourceId: [event.sourceId, nested.sourceId],
    resourceKind: [event.resourceType, nested.resourceType, event.type],
  };
  return {
    error: isRecord(event.error) ? event.error : event.error ?? event,
    status: finiteNumber(event.status) ?? finiteNumber(nested.status),
    url: firstString(...fields.url), sourceId: firstString(...fields.sourceId), resourceKind: firstString(...fields.resourceKind),
  };
}


function firstString(...values: unknown[]): string | undefined {
  return values.find((value): value is string => typeof value === 'string' && value.length > 0);
}

function finiteNumber(value: unknown): number | undefined {
  return typeof value === 'number' && Number.isFinite(value) ? value : undefined;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

function syncActivePoint(
  map: MapApi,
  point: LatLng | null | undefined,
  lastPointRef: React.MutableRefObject<LatLng | null>,
) {
  const layerExists = Boolean(map.getLayer(ACTIVE_POINT_LAYER_ID));
  if (point == null) {
    lastPointRef.current = null;
    if (layerExists) map.removeLayer(ACTIVE_POINT_LAYER_ID);
    if (map.getSource(ACTIVE_POINT_SOURCE_ID)) map.removeSource(ACTIVE_POINT_SOURCE_ID);
    return;
  }
  const sameLocation = lastPointRef.current?.lat === point.lat && lastPointRef.current?.lng === point.lng;
  const paint = {
    'circle-radius': 6,
    'circle-color': getCssColor('--rm-accent', FALLBACK_ACTIVE_POINT_COLOR),
    'circle-stroke-width': 2,
    'circle-stroke-color': getCssColor('--rm-bg-surface', CHART_COLORS.muted),
  };
  if (layerExists && sameLocation) {
    for (const key of ['circle-color', 'circle-stroke-color'] as const) map.setPaintProperty(ACTIVE_POINT_LAYER_ID, key, paint[key]);
    return;
  }
  lastPointRef.current = point;
  updateMapSource(map, ACTIVE_POINT_SOURCE_ID, { type: 'Feature', geometry: { type: 'Point', coordinates: [point.lng, point.lat] }, properties: {} });
  if (!layerExists) map.addLayer({ id: ACTIVE_POINT_LAYER_ID, type: 'circle', source: ACTIVE_POINT_SOURCE_ID, paint });
}


function applyPerspective(map: MapApi, basemap: BasemapConfig, preference: MapStylePreference) {
  const perspective = resolveStyleDescriptor(basemap, preference, 'light').descriptor?.perspective_3d === true;
  const camera = perspective ? { pitch: 45, bearing: -15, rotate: 'enable' as const } : { pitch: 0, bearing: 0, rotate: 'disable' as const };
  map.setPitch?.(camera.pitch);
  map.setBearing?.(camera.bearing);
  map.dragRotate?.[camera.rotate]();
}


function getCssColor(variableName: string, fallbackColor: string) {
  if (typeof document === 'undefined') return fallbackColor;

  const color = getComputedStyle(document.documentElement).getPropertyValue(variableName).trim();
  return color || fallbackColor;
}

function serializePoint(point: LatLng | undefined) {
  return point ? `${point.lat.toFixed(5)},${point.lng.toFixed(5)}` : 'none';
}
