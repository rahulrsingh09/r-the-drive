import type { BasemapConfig } from './TripMapChart';

/**
 * OpenFreeMap Liberty for the R maps. Light mode uses Liberty as published.
 * Dark mode uses the same tiles with Liberty recolored into the R dark theme
 * (`darkLiberty`). Roads stay neutral so the route line stands out.
 *
 * The browser never contacts tiles.openfreemap.org directly: the app CSP allows
 * only same-origin requests, so the style comes through the API proxy that the
 * basemap config names, and dependent URLs point at the reserved proxy origin.
 */
export const OPENFREEMAP_LIBERTY_STYLE_ID = 'liberty';
export const BASEMAP_PROXY_PLACEHOLDER_ORIGIN = 'https://panda.invalid';
const BASEMAP_PROXY_PATH = '/v1/external/basemap/';

/** Map data colors for the dark Liberty style. These recolor map data, not app chrome. */
export const DARK_LIBERTY_COLORS = {
  bg: '#12161b', land: '#151a20', res: '#181d24', park: '#16301f', wood: '#173322', grass: '#1a3324', water: '#1b4166', river: '#2d6aa0',
  bld: '#222932', mway: '#9aa7b6', major: '#7b8796', minor: '#4a5560', path: '#3a4450', casing: '#0d1014', rail: '#5a6470', bound: '#5d6b7c',
  label: '#e3e8ee', city: '#f4f7fa', waterLbl: '#8fc1ec', halo: '#0d1014', aero: '#1f252d',
} as const;

interface StyleLayer {
  id: string;
  type: string;
  paint?: Record<string, unknown>;
  [key: string]: unknown;
}

export interface MapLibreStyleJson {
  version: number;
  layers: StyleLayer[];
  [key: string]: unknown;
}

/** Recolors the Liberty style into the R dark theme. Returns a copy; the input is not changed. */
export function darkLiberty<T extends MapLibreStyleJson>(source: T): T {
  const style = JSON.parse(JSON.stringify(source)) as T;
  const C = DARK_LIBERTY_COLORS;
  for (const layer of style.layers) {
    const id = layer.id;
    const paint = layer.paint = layer.paint ?? {};
    if (layer.type === 'background') paint['background-color'] = C.bg;
    else if (layer.type === 'raster') {
      paint['raster-opacity'] = 0.35; paint['raster-brightness-max'] = 0.35; paint['raster-saturation'] = -0.4;
    } else if (layer.type === 'fill') {
      const color = /water/.test(id) ? C.water : /park/.test(id) ? C.park : /wood/.test(id) ? C.wood
        : /grass|wetland|pitch|cemetery/.test(id) ? C.grass : /building/.test(id) ? C.bld : /residential/.test(id) ? C.res
          : /aeroway|runway|taxiway|sand|ice/.test(id) ? C.aero : C.land;
      paint['fill-color'] = color;
      paint['fill-outline-color'] = color;
      if (/park_outline/.test(id)) paint['fill-opacity'] = 0;
      delete paint['fill-pattern'];
      if (/road_area_pattern/.test(id)) paint['fill-opacity'] = 0;
    } else if (layer.type === 'fill-extrusion') {
      paint['fill-extrusion-color'] = C.bld; paint['fill-extrusion-opacity'] = 0.7;
    } else if (layer.type === 'line') {
      paint['line-color'] = /waterway/.test(id) ? C.river : /casing/.test(id) ? C.casing : /boundary/.test(id) ? C.bound
        : /rail/.test(id) ? C.rail : /motorway/.test(id) ? C.mway : /trunk|primary|secondary|tertiary/.test(id) ? C.major
          : /path|pedestrian|track|service/.test(id) ? C.path : /park_outline/.test(id) ? C.park : C.minor;
      delete paint['line-pattern'];
      if (/park/.test(id)) paint['line-opacity'] = 0;
    } else if (layer.type === 'symbol') {
      paint['text-color'] = /water/.test(id) ? C.waterLbl : /label_(city|state|country|town)/.test(id) ? C.city : C.label;
      paint['text-halo-color'] = C.halo;
      paint['text-halo-width'] = 1.4;
      if (/poi/.test(id)) paint['icon-opacity'] = 0.75;
    }
  }
  return style;
}

/** The proxied Liberty style URL from the basemap config, or null when OpenFreeMap is not active. */
export function libertyStyleUrl(config: BasemapConfig | null | undefined): string | null {
  if (!config?.enabled) return null;
  const descriptor = config.styles?.find((style) => style.id === OPENFREEMAP_LIBERTY_STYLE_ID && style.kind === 'style');
  return descriptor?.light_url || null;
}

/**
 * Rewrites a proxied basemap request to the current origin and adds the bearer
 * token. Other URLs pass through unchanged.
 */
export function basemapRequest(url: string, token: string | null, origin = typeof window === 'undefined' ? 'http://localhost' : window.location.origin) {
  try {
    const requestUrl = new URL(url, origin);
    const placeholder = requestUrl.origin === BASEMAP_PROXY_PLACEHOLDER_ORIGIN && requestUrl.pathname.startsWith(BASEMAP_PROXY_PATH);
    const sameOrigin = requestUrl.origin === origin && requestUrl.pathname.startsWith(BASEMAP_PROXY_PATH);
    if (placeholder || sameOrigin) {
      const firstPartyUrl = placeholder ? `${origin}${requestUrl.pathname}${requestUrl.search}${requestUrl.hash}` : requestUrl.toString();
      return token
        ? { url: firstPartyUrl, headers: { Authorization: `Bearer ${token}` }, credentials: 'same-origin' as const }
        : { url: firstPartyUrl, credentials: 'same-origin' as const };
    }
  } catch {
    // MapLibre reports malformed URLs through its own error event.
  }
  return { url };
}

const styleCache = new Map<string, Promise<MapLibreStyleJson>>();

/** Loads the Liberty style once per URL and mode. Dark mode applies `darkLiberty`. */
export function loadLibertyStyle(url: string, mode: 'light' | 'dark', token: string | null, fetcher: typeof fetch = fetch) {
  const key = `${mode}|${url}`;
  let pending = styleCache.get(key);
  if (!pending) {
    const request = basemapRequest(url, token);
    pending = fetcher(request.url, 'headers' in request ? { headers: request.headers, credentials: 'same-origin' } : { credentials: 'same-origin' })
      .then((response) => {
        if (!response.ok) throw new Error(`Basemap style request failed with ${response.status}`);
        return response.json() as Promise<MapLibreStyleJson>;
      })
      .then((json) => (mode === 'dark' ? darkLiberty(json) : json));
    pending.catch(() => styleCache.delete(key));
    styleCache.set(key, pending);
  }
  return pending;
}

/** A plain background style. The route still draws when no basemap is available. */
export function neutralStyle(background: string): MapLibreStyleJson {
  return { version: 8, sources: {}, layers: [{ id: 'neutral-background', type: 'background', paint: { 'background-color': background } }] };
}
