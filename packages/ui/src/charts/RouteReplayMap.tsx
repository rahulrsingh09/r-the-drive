import * as React from 'react';
import { loadMapLibre, type BasemapConfig } from './TripMapChart';
import { basemapRequest, libertyStyleUrl, loadLibertyStyle, neutralStyle, type MapLibreStyleJson } from './openFreeMap';
import { reportClientError } from '../lib/clientDiagnostics';

/** A route vertex: [longitude, latitude]. */
export type RouteCoordinate = [number, number];

export interface RouteReplayMapProps {
  route: RouteCoordinate[];
  /**
   * Replay position as a fractional route vertex index. Null, 0 or the last
   * index shows the whole route without the moving marker.
   */
  progress?: number | null;
  mode?: 'light' | 'dark';
  basemapConfig?: BasemapConfig | null | undefined;
  /** Bearer token for the same-origin basemap proxy. */
  accessToken?: string | null;
  height?: number | string;
  className?: string;
  /** Accessible name of the map region. */
  label?: string;
  mapLoader?: typeof loadMapLibre;
  styleLoader?: typeof loadLibertyStyle;
}

interface MarkerApi { setLngLat(point: RouteCoordinate): MarkerApi; addTo(map: unknown): MarkerApi; remove(): void }
interface ReplayMapApi {
  on(event: string, cb: (event?: unknown) => void): void;
  addControl(control: unknown, position?: string): void;
  addSource(id: string, source: unknown): void;
  addLayer(layer: unknown): void;
  getSource(id: string): { setData(data: unknown): void } | undefined;
  getLayer(id: string): unknown;
  setPaintProperty(layer: string, name: string, value: unknown): void;
  resize(): void;
  remove(): void;
  touchZoomRotate?: { disableRotation(): void };
}

const ATTRIBUTION = [
  { label: 'OpenFreeMap', url: 'https://openfreemap.org' },
  { label: '© OpenMapTiles', url: 'https://www.openmaptiles.org/' },
  { label: '© OpenStreetMap', url: 'https://www.openstreetmap.org/copyright' },
];

function cssColor(name: string, fallback: string) {
  if (typeof window === 'undefined') return fallback;
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim() || fallback;
}

function lineFeature(coordinates: RouteCoordinate[]) {
  return { type: 'Feature', properties: {}, geometry: { type: 'LineString', coordinates } };
}

/** Point at a fractional vertex index along the route. */
export function routePointAt(route: RouteCoordinate[], fraction: number): RouteCoordinate {
  const last = route.length - 1;
  const index = Math.max(0, Math.min(last, Math.floor(fraction)));
  const next = Math.min(last, index + 1);
  const t = Math.max(0, Math.min(1, fraction - index));
  const a = route[index]!;
  const b = route[next]!;
  return [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
}

function bounds(route: RouteCoordinate[]): [[number, number], [number, number]] {
  const lngs = route.map((point) => point[0]);
  const lats = route.map((point) => point[1]);
  return [[Math.min(...lngs), Math.min(...lats)], [Math.max(...lngs), Math.max(...lats)]];
}

/**
 * Trip route on MapLibre with OpenFreeMap Liberty (dark mode: recolored
 * Liberty). Two-finger or Ctrl+scroll gestures move the map, so the page
 * scrolls normally. A moving marker and a drawn line follow `progress`.
 */
export function RouteReplayMap({
  route, progress = null, mode = 'dark', basemapConfig, accessToken = null, height = 320, className, label = 'Trip route map',
  mapLoader = loadMapLibre, styleLoader = loadLibertyStyle,
}: RouteReplayMapProps) {
  const containerRef = React.useRef<HTMLDivElement>(null);
  const mapRef = React.useRef<ReplayMapApi | null>(null);
  const markerRef = React.useRef<MarkerApi | null>(null);
  const loadedRef = React.useRef(false);
  const shownRef = React.useRef(false);
  const progressRef = React.useRef(progress);
  const tokenRef = React.useRef(accessToken);
  tokenRef.current = accessToken;
  const [failed, setFailed] = React.useState(false);
  const routeKey = route.length > 1 ? `${route.length}:${route[0]!.join(',')}:${route.at(-1)!.join(',')}` : '';
  const styleUrl = libertyStyleUrl(basemapConfig);

  const applyProgress = React.useCallback(() => {
    const map = mapRef.current;
    if (!map || !loadedRef.current || route.length < 2) return;
    const value = progressRef.current;
    const last = route.length - 1;
    const playing = value != null && value > 0 && value < last;
    if (map.getLayer('replay-route-line')) map.setPaintProperty('replay-route-line', 'line-opacity', playing ? 0.35 : 1);
    const head = playing ? routePointAt(route, value) : route[0]!;
    const done = playing ? [...route.slice(0, Math.floor(value) + 1), head] : [route[0]!, route[0]!];
    map.getSource('replay-done')?.setData(lineFeature(done));
    const marker = markerRef.current;
    if (!marker) return;
    if (playing) {
      marker.setLngLat(head);
      if (!shownRef.current) { marker.addTo(map); shownRef.current = true; }
    } else if (shownRef.current) {
      marker.remove();
      shownRef.current = false;
    }
  }, [route]);

  React.useEffect(() => {
    progressRef.current = progress;
    applyProgress();
  }, [progress, applyProgress]);

  React.useEffect(() => {
    const element = containerRef.current;
    if (!element || route.length < 2) return undefined;
    let cancelled = false;
    setFailed(false);
    const fail = (error: unknown, operation: string) => {
      if (cancelled) return;
      reportClientError(error, { event: 'maplibre.route_replay_failed', area: 'map', operation, severity: 'error' });
      setFailed(true);
    };
    (async () => {
      try {
        const background = cssColor('--rm-bg-elevated', mode === 'dark' ? '#1b1e1d' : '#e5ede7');
        const stylePromise: Promise<MapLibreStyleJson> = styleUrl
          ? styleLoader(styleUrl, mode, tokenRef.current).catch(() => neutralStyle(background))
          : Promise.resolve(neutralStyle(background));
        const [maplibregl, style] = await Promise.all([mapLoader(), stylePromise]);
        if (cancelled || !containerRef.current) return;
        const map = new maplibregl.Map({
          container: containerRef.current,
          style: style as never,
          bounds: bounds(route),
          fitBoundsOptions: { padding: 44, maxZoom: 14 },
          attributionControl: false,
          cooperativeGestures: true,
          dragRotate: false,
          pitchWithRotate: false,
          touchPitch: false,
          maxZoom: 17,
          transformRequest: (url: string) => basemapRequest(url, tokenRef.current),
        }) as unknown as ReplayMapApi;
        mapRef.current = map;
        map.addControl(new maplibregl.NavigationControl({ showCompass: false }), 'top-right');
        map.touchZoomRotate?.disableRotation();
        const markerElement = document.createElement('div');
        markerElement.className = 'rm-replay-marker';
        markerElement.setAttribute('aria-hidden', 'true');
        markerRef.current = new maplibregl.Marker({ element: markerElement }).setLngLat(route[0]!) as unknown as MarkerApi;
        map.on('error', (event) => {
          const details = event as { error?: unknown } | undefined;
          reportClientError(details?.error ?? event, { event: 'maplibre.error', area: 'map', operation: 'route-replay-resource', severity: 'error' });
        });
        map.on('load', () => {
          if (cancelled) return;
          const accent = cssColor('--rm-chart-accent', '#f2c14e');
          const ink = cssColor('--rm-bg-surface', mode === 'dark' ? '#101212' : '#ffffff');
          map.addSource('replay-route', { type: 'geojson', data: lineFeature(route) });
          map.addSource('replay-done', { type: 'geojson', data: lineFeature([route[0]!, route[0]!]) });
          map.addSource('replay-ends', {
            type: 'geojson',
            data: {
              type: 'FeatureCollection',
              features: [
                { type: 'Feature', properties: { k: 's' }, geometry: { type: 'Point', coordinates: route[0] } },
                { type: 'Feature', properties: { k: 'e' }, geometry: { type: 'Point', coordinates: route.at(-1) } },
              ],
            },
          });
          const lineLayout = { 'line-cap': 'round', 'line-join': 'round' };
          map.addLayer({ id: 'replay-route-case', type: 'line', source: 'replay-route', layout: lineLayout, paint: { 'line-color': ink, 'line-width': 9, 'line-opacity': 0.7 } });
          map.addLayer({ id: 'replay-route-line', type: 'line', source: 'replay-route', layout: lineLayout, paint: { 'line-color': accent, 'line-width': 5, 'line-opacity': 1 } });
          map.addLayer({ id: 'replay-done-line', type: 'line', source: 'replay-done', layout: lineLayout, paint: { 'line-color': accent, 'line-width': 6 } });
          map.addLayer({
            id: 'replay-ends', type: 'circle', source: 'replay-ends',
            paint: {
              'circle-radius': 7,
              'circle-color': ['match', ['get', 'k'], 's', ink, accent],
              'circle-stroke-width': 3,
              'circle-stroke-color': ['match', ['get', 'k'], 's', accent, ink],
            },
          });
          loadedRef.current = true;
          applyProgress();
        });
      } catch (error) {
        fail(error, 'route-replay-initialize');
      }
    })();
    const observer = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(() => mapRef.current?.resize());
    observer?.observe(element);
    return () => {
      cancelled = true;
      observer?.disconnect();
      loadedRef.current = false;
      shownRef.current = false;
      markerRef.current?.remove();
      markerRef.current = null;
      try { mapRef.current?.remove(); } catch { /* The map may already be gone. */ }
      mapRef.current = null;
    };
    // The route key and style identify the map; the route array is read through the closure.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [routeKey, mode, styleUrl, mapLoader, styleLoader]);

  return (
    <div
      className={className}
      data-route-replay-map
      role="region"
      aria-label={label}
      style={{ position: 'relative', overflow: 'hidden', height }}
    >
      <div ref={containerRef} style={{ position: 'absolute', inset: 0 }} />
      {failed || route.length < 2 ? (
        <p data-map-error style={{ position: 'absolute', inset: 0, display: 'grid', placeItems: 'center', margin: 0, color: 'var(--rm-text-secondary)', fontSize: 13 }}>
          {route.length < 2 ? 'No route was recorded for this trip.' : 'The map could not load.'}
        </p>
      ) : null}
      {styleUrl ? <p
        data-map-attribution
        style={{
          position: 'absolute', zIndex: 2, left: 8, bottom: 6, margin: 0, fontSize: 10, lineHeight: 1.4, padding: '2px 7px', borderRadius: 6,
          color: 'var(--rm-text-secondary)', background: 'color-mix(in srgb, var(--rm-bg-surface) 78%, transparent)',
        }}
      >
        {ATTRIBUTION.map((item, index) => (
          <React.Fragment key={item.url}>
            {index ? ' · ' : null}
            <a href={item.url} target="_blank" rel="noopener noreferrer" style={{ color: 'inherit', textDecoration: 'none' }}>{item.label}</a>
          </React.Fragment>
        ))}
      </p> : null}
    </div>
  );
}
