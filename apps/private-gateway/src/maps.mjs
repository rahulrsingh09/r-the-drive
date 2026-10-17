import { readBoundedBytes } from './bounded-body.mjs';

const PROVIDER = 'https://tiles.openfreemap.org';
export const MAP_PREFIX = '/v1/external/basemap/openfreemap/';
export const MAP_CONFIG = '/v1/external/basemap/config';
export const CONNECTIONS = '/v1/settings/external-connections';
const REVISION = 'gateway-openfreemap-1';
const STYLES = ['positron', 'bright', 'liberty', 'dark', 'fiord'];
const MAX_BYTES = 4 * 1024 * 1024;
const MEDIA = {
  style: ['application/json'], tilejson: ['application/json'], sprite: ['application/json'],
  png: ['image/png'], pbf: ['application/x-protobuf', 'application/vnd.mapbox-vector-tile', 'application/octet-stream'],
};
const MAP_CONNECTION = {
  name: 'OpenFreeMap', enabled: true, editable: false, mode: 'remote',
  basemap_provider: 'openfreemap', endpoint: PROVIDER, endpoint_is_private: false,
  purpose: 'Street maps through the private gateway. Managed by the deployment.',
  execution: 'Cloudflare gateway', privacy_url: 'https://openfreemap.org/privacy/',
  terms_url: 'https://openfreemap.org/tos/', disabled_effect: 'Map availability is managed by the gateway deployment.',
  has_api_key: false, has_bearer_token: false, base_url: null, light_url_template: null,
  dark_url_template: null, allow_private_network: false, cache: null, last_error: null,
  request_count_today: 0, observed_health: null, observed_error: null,
};

export function basemapConfiguration() {
  const variants = [['follow-theme', 'Follow appearance', 'positron', 'dark', false]];
  for (const style of STYLES) variants.push([style, style[0].toUpperCase() + style.slice(1), style, style, false]);
  variants.push(['3d', '3D', 'liberty', 'liberty', true]);
  const styles = [];
  for (const [id, label, light, dark, perspective_3d] of variants) {
    styles.push({ id, label, kind: 'style', light_url: `${MAP_PREFIX}styles/${light}?v=${REVISION}`,
      dark_url: `${MAP_PREFIX}styles/${dark}?v=${REVISION}`, perspective_3d });
  }
  return {
    enabled: true, provider_preference: 'openfreemap', resolved_provider: 'openfreemap', revision: REVISION,
    styles, attributions: [
      { label: 'OpenFreeMap', url: 'https://openfreemap.org/' },
      { label: '© OpenMapTiles', url: 'https://openmaptiles.org/' },
      { label: 'Data from OpenStreetMap', url: 'https://www.openstreetmap.org/copyright' },
    ],
  };
}

export function describeGatewayMaps(inventory) {
  if (!Array.isArray(inventory.connections)) throw new Error('Invalid connection inventory');
  const connections = [];
  for (const item of inventory.connections) {
    connections.push(item.id === 'basemap' ? {
      ...item, ...MAP_CONNECTION,
      data_shared: ['Requested map areas', 'Network metadata (Cloudflare may forward viewer IP)'],
      private_network_allowlist: [],
    } : item);
  }
  return { ...inventory, connections };
}

function tile(z, x, y, maxZoom) {
  for (const coordinate of [z, x, y]) if (!/^(0|[1-9][0-9]{0,5})$/.test(coordinate)) return false;
  const zoom = Number(z);
  if (zoom > maxZoom) return false;
  const width = 2 ** zoom;
  return Number(x) < width && Number(y) < width;
}

export function resourceKind(resource) {
  if (resource.length > 512 || /[\\?#%]|\.{2}|[\x00-\x1f\x7f]/.test(resource)) return null;
  if (resource === 'planet') return 'tilejson';
  const style = /^styles\/([^/]+)$/.exec(resource);
  if (style) return STYLES.includes(style[1]) ? 'style' : null;
  const vector = /^planet\/[0-9]{8}_[0-9]{6}_pt\/([^/]+)\/([^/]+)\/([^/]+)\.pbf$/.exec(resource);
  if (vector) return tile(vector[1], vector[2], vector[3], 14) ? 'pbf' : null;
  const raster = /^natural_earth\/ne2sr\/([^/]+)\/([^/]+)\/([^/]+)\.png$/.exec(resource);
  if (raster) return tile(raster[1], raster[2], raster[3], 6) ? 'png' : null;
  const sprite = /^sprites\/ofm_[a-zA-Z0-9]{1,32}\/ofm(?:@2x)?\.(json|png)$/.exec(resource);
  if (sprite) return sprite[1] === 'json' ? 'sprite' : 'png';
  const glyph = /^fonts\/[a-zA-Z0-9_, -]{1,192}\/([0-9]{1,5})-([0-9]{1,5})\.pbf$/.exec(resource);
  if (!glyph) return null;
  const start = Number(glyph[1]);
  const end = Number(glyph[2]);
  return start % 256 === 0 && end === start + 255 && end <= 65535 ? 'pbf' : null;
}

function proxyResource(value) {
  if (typeof value !== 'string') throw new Error('Invalid map URL');
  let resource;
  for (const allowed of [PROVIDER, 'https://__TILEJSON_DOMAIN__']) {
    if (value.startsWith(allowed + '/')) { resource = value.substring(allowed.length + 1); break; }
  }
  if (resource === undefined) throw new Error('Unexpected map destination');
  const substitutions = { z: '0', x: '0', y: '0', fontstack: 'Noto Sans Regular', range: '0-255' };
  const concrete = resource.replace(/\{(z|x|y|fontstack|range)\}/g, (_, name) => substitutions[name]);
  if (resourceKind(concrete) === null && resourceKind(concrete + '.json') === null) throw new Error('Unexpected map resource');
  return `https://panda.invalid${MAP_PREFIX}${resource}`;
}

function mapSource(source) {
  if (!source || !['vector', 'raster'].includes(source.type) || source.data) throw new Error('Invalid map source');
  if (!source.url && !source.tiles) throw new Error('Missing map source');
  const result = { ...source };
  if (source.url) result.url = proxyResource(source.url);
  if (source.tiles) result.tiles = source.tiles.map(proxyResource);
  return result;
}

function rewriteJson(bytes, kind) {
  const data = JSON.parse(new TextDecoder().decode(bytes));
  if (!data || typeof data !== 'object' || Array.isArray(data)) throw new Error('Invalid map JSON');
  switch (kind) {
    case 'style': {
      const valid = data.version === 8 && Array.isArray(data.layers) && data.layers.length <= 2048
        && data.sources && typeof data.sources === 'object' && !Array.isArray(data.sources)
        && Object.keys(data.sources).length <= 16 && !data.imports;
      if (!valid) throw new Error('Invalid map style');
      const sources = {};
      for (const [id, source] of Object.entries(data.sources)) {
        Object.defineProperty(sources, id, { value: mapSource(source), enumerable: true, configurable: true, writable: true });
      }
      const result = { ...data, sources };
      for (const field of ['sprite', 'glyphs']) if (data[field]) result[field] = proxyResource(data[field]);
      return JSON.stringify(result);
    }
    case 'tilejson':
      if (!Array.isArray(data.tiles) || data.tiles.length < 1 || data.tiles.length > 4 || data.grids || data.data) throw new Error('Invalid tile manifest');
      return JSON.stringify({ ...data, tiles: data.tiles.map(proxyResource) });
    default: return JSON.stringify(data);
  }
}

async function limitedBody(response, limit) {
  const declared = Number(response.headers.get('Content-Length'));
  if (declared <= limit || Number.isNaN(declared)) {
    const bytes = await readBoundedBytes(response.body, limit);
    if (bytes) return bytes;
    throw new Error('Empty map response');
  }
  await response.body?.cancel();
  throw new Error('Map file too large');
}

function failure(status) {
  const headers = new Headers({ 'Cache-Control': 'no-store', 'X-Content-Type-Options': 'nosniff' });
  headers.set('Content-Type', 'text/plain; charset=utf-8');
  return new Response('Map tiles unavailable. Your recorded route is retained.', { headers, status });
}

function resourceRequest(request) {
  if (request.method !== 'GET') return { error: 405 };
  const url = new URL(request.url);
  for (const [key, value] of url.searchParams) {
    if ((key !== 'v' && key !== 'cf') || !/^[a-zA-Z0-9_-]{1,80}$/.test(value)) return { error: 400 };
  }
  let resource;
  try { resource = decodeURIComponent(url.pathname.slice(MAP_PREFIX.length)); }
  catch { return { error: 400 }; }
  const kind = resourceKind(resource);
  if (!kind) return { error: 404 };
  const target = new URL(PROVIDER);
  target.pathname = '/' + resource.split('/').map(encodeURIComponent).join('/');
  return { target, kind };
}

async function fetchResource(target, kind, mapFetch) {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), 10_000);
  try {
    const request = new Request(target, { redirect: 'manual', credentials: 'omit', headers: { Accept: '*/*' }, signal: controller.signal });
    const response = await mapFetch(request);
    const contentType = (response.headers.get('Content-Type') ?? '').split(';')[0].trim();
    if (response.status !== 200 || !MEDIA[kind].includes(contentType)) {
      await response.body?.cancel();
      throw new Error('Map request failed');
    }
    const bytes = await limitedBody(response, kind === 'style' ? 512 * 1024 : MAX_BYTES);
    if (bytes.length === 0) throw new Error('Empty map file');
    if (kind === 'png') {
      const signature = [137, 80, 78, 71, 13, 10, 26, 10];
      for (let i = 0; i < signature.length; i++) if (bytes[i] !== signature[i]) throw new Error('Invalid map image');
    }
    return MEDIA[kind][0] === 'application/json' ? rewriteJson(bytes, kind) : bytes;
  } finally { clearTimeout(timeout); }
}

// The gateway verifies Access before selecting this public-cartography handler.
export async function mapResource(request, mapFetch = fetch) {
  const plan = resourceRequest(request);
  if (plan.error) return failure(plan.error);
  try {
    const body = await fetchResource(plan.target, plan.kind, mapFetch);
    const maxAge = plan.kind === 'style' || plan.kind === 'tilejson' ? 300 : 86400;
    return new Response(body, { headers: {
      'Content-Type': MEDIA[plan.kind][0], 'Cache-Control': `private, max-age=${maxAge}`,
      'CDN-Cache-Control': 'no-store', 'X-Content-Type-Options': 'nosniff',
      'Referrer-Policy': 'no-referrer', 'Content-Security-Policy': "default-src 'none'; frame-ancestors 'none'",
    } });
  } catch { return failure(502); }
}
