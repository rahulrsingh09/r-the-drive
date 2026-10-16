const PROVIDER = 'https://tiles.openfreemap.org';
export const MAP_PREFIX = '/v1/external/basemap/openfreemap/';
export const MAP_CONFIG = '/v1/external/basemap/config';
export const CONNECTIONS = '/v1/settings/external-connections';
const REVISION = 'gateway-openfreemap-1';
const STYLES = ['positron', 'bright', 'liberty', 'dark', 'fiord'];
const MAX_BYTES = 4 * 1024 * 1024;

export function basemapConfiguration() {
  const styleUrl = (style) => `${MAP_PREFIX}styles/${style}?v=${REVISION}`;
  const styles = STYLES.map((id) => ({
    id,
    label: id[0].toUpperCase() + id.slice(1),
    kind: 'style',
    light_url: styleUrl(id),
    dark_url: styleUrl(id),
    perspective_3d: false,
  }));
  styles.unshift({
    id: 'follow-theme',
    label: 'Follow appearance',
    kind: 'style',
    light_url: styleUrl('positron'),
    dark_url: styleUrl('dark'),
    perspective_3d: false,
  });
  styles.push({
    ...styles.find((style) => style.id === 'liberty'),
    id: '3d',
    label: '3D',
    perspective_3d: true,
  });
  return {
    enabled: true,
    provider_preference: 'openfreemap',
    resolved_provider: 'openfreemap',
    revision: REVISION,
    styles,
    attributions: [
      { label: 'OpenFreeMap', url: 'https://openfreemap.org/' },
      { label: '© OpenMapTiles', url: 'https://openmaptiles.org/' },
      { label: 'Data from OpenStreetMap', url: 'https://www.openstreetmap.org/copyright' },
    ],
  };
}

export function describeGatewayMaps(inventory) {
  if (!Array.isArray(inventory.connections)) throw new Error('Invalid connection inventory');
  return {
    ...inventory,
    connections: inventory.connections.map((item) =>
      item.id !== 'basemap'
        ? item
        : {
            ...item,
            name: 'OpenFreeMap',
            enabled: true,
            editable: false,
            mode: 'remote',
            basemap_provider: 'openfreemap',
            endpoint: PROVIDER,
            endpoint_is_private: false,
            purpose: 'Street maps through the private gateway. Managed by the deployment.',
            data_shared: [
              'Requested map areas',
              'Network metadata (Cloudflare may forward viewer IP)',
            ],
            execution: 'Cloudflare gateway',
            privacy_url: 'https://openfreemap.org/privacy/',
            terms_url: 'https://openfreemap.org/tos/',
            disabled_effect: 'Map availability is managed by the gateway deployment.',
            has_api_key: false,
            has_bearer_token: false,
            base_url: null,
            light_url_template: null,
            dark_url_template: null,
            allow_private_network: false,
            private_network_allowlist: [],
            cache: null,
            last_error: null,
            request_count_today: 0,
            observed_health: null,
            observed_error: null,
          }
    ),
  };
}

function tile(z, x, y, maxZoom) {
  return (
    [z, x, y].every((part) => /^(0|[1-9][0-9]{0,5})$/.test(part)) &&
    Number(z) <= maxZoom &&
    Number(x) < 2 ** Number(z) &&
    Number(y) < 2 ** Number(z)
  );
}

export function resourceKind(resource) {
  if (resource.length > 512 || /[\\?#%]|\.{2}|[\x00-\x1f\x7f]/.test(resource)) return null;
  const parts = resource.split('/');
  if (parts.length === 2 && parts[0] === 'styles' && STYLES.includes(parts[1])) return 'style';
  if (resource === 'planet') return 'tilejson';
  if (
    parts.length === 5 &&
    parts[0] === 'planet' &&
    /^[0-9]{8}_[0-9]{6}_pt$/.test(parts[1]) &&
    parts[4].endsWith('.pbf') &&
    tile(parts[2], parts[3], parts[4].slice(0, -4), 14)
  )
    return 'pbf';
  if (
    parts.length === 5 &&
    parts[0] === 'natural_earth' &&
    parts[1] === 'ne2sr' &&
    parts[4].endsWith('.png') &&
    tile(parts[2], parts[3], parts[4].slice(0, -4), 6)
  )
    return 'png';
  if (
    parts.length === 3 &&
    parts[0] === 'sprites' &&
    /^ofm_[a-zA-Z0-9]{1,32}$/.test(parts[1]) &&
    /^ofm(?:@2x)?\.(?:json|png)$/.test(parts[2])
  )
    return parts[2].endsWith('.json') ? 'sprite' : 'png';
  if (parts.length === 3 && parts[0] === 'fonts' && /^[a-zA-Z0-9_, -]{1,192}$/.test(parts[1])) {
    const range = /^([0-9]{1,5})-([0-9]{1,5})\.pbf$/.exec(parts[2]);
    if (
      range &&
      Number(range[1]) % 256 === 0 &&
      Number(range[2]) === Number(range[1]) + 255 &&
      Number(range[2]) <= 65535
    )
      return 'pbf';
  }
  return null;
}

function proxyResource(value) {
  if (typeof value !== 'string') throw new Error('Invalid map URL');
  const prefix = [PROVIDER + '/', 'https://__TILEJSON_DOMAIN__/'].find((candidate) =>
    value.startsWith(candidate)
  );
  if (!prefix) throw new Error('Unexpected map destination');
  const resource = value.slice(prefix.length);
  const example = resource
    .replaceAll('{z}', '0')
    .replaceAll('{x}', '0')
    .replaceAll('{y}', '0')
    .replaceAll('{fontstack}', 'Noto Sans Regular')
    .replaceAll('{range}', '0-255');
  if (!resourceKind(example) && !resourceKind(example + '.json'))
    throw new Error('Unexpected map resource');
  return `https://panda.invalid${MAP_PREFIX}${resource}`;
}

function rewriteJson(bytes, kind) {
  const data = JSON.parse(new TextDecoder().decode(bytes));
  if (!data || Array.isArray(data) || typeof data !== 'object') throw new Error('Invalid map JSON');
  if (kind === 'style') {
    if (
      data.version !== 8 ||
      !Array.isArray(data.layers) ||
      data.layers.length > 2048 ||
      !data.sources ||
      typeof data.sources !== 'object' ||
      Array.isArray(data.sources) ||
      Object.keys(data.sources).length > 16 ||
      data.imports
    )
      throw new Error('Invalid map style');
    for (const source of Object.values(data.sources)) {
      if (!source || !['vector', 'raster'].includes(source.type) || source.data)
        throw new Error('Invalid map source');
      if (source.url) source.url = proxyResource(source.url);
      if (source.tiles) source.tiles = source.tiles.map(proxyResource);
      if (!source.url && !source.tiles) throw new Error('Missing map source');
    }
    if (data.sprite) data.sprite = proxyResource(data.sprite);
    if (data.glyphs) data.glyphs = proxyResource(data.glyphs);
  } else if (kind === 'tilejson') {
    if (!Array.isArray(data.tiles) || !data.tiles.length || data.tiles.length > 4)
      throw new Error('Invalid tile manifest');
    data.tiles = data.tiles.map(proxyResource);
    if (data.grids || data.data) throw new Error('Unexpected tile resources');
  }
  return JSON.stringify(data);
}

async function limitedBody(response, limit) {
  if (Number(response.headers.get('Content-Length')) > limit) {
    await response.body?.cancel();
    throw new Error('Map file too large');
  }
  const reader = response.body?.getReader();
  if (!reader) throw new Error('Empty map response');
  const chunks = [];
  let size = 0;
  try {
    while (true) {
      const { value, done } = await reader.read();
      if (done) break;
      size += value.byteLength;
      if (size > limit) throw new Error('Map file too large');
      chunks.push(value);
    }
  } catch (error) {
    await reader.cancel();
    throw error;
  }
  const body = new Uint8Array(size);
  let offset = 0;
  for (const chunk of chunks) {
    body.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return body;
}

function failure(status) {
  return new Response('Map tiles unavailable. Your recorded route is retained.', {
    status,
    headers: {
      'Content-Type': 'text/plain; charset=utf-8',
      'Cache-Control': 'no-store',
      'X-Content-Type-Options': 'nosniff',
    },
  });
}

// Only public cartography uses this path; the caller must first verify Cloudflare Access.
export async function mapResource(request, mapFetch = fetch) {
  if (request.method !== 'GET') return failure(405);
  const url = new URL(request.url);
  if (
    [...url.searchParams].some(
      ([key, value]) => !['v', 'cf'].includes(key) || !/^[a-zA-Z0-9_-]{1,80}$/.test(value)
    )
  )
    return failure(400);
  let resource;
  try {
    resource = decodeURIComponent(url.pathname.slice(MAP_PREFIX.length));
  } catch {
    return failure(400);
  }
  const kind = resourceKind(resource);
  if (!kind) return failure(404);
  try {
    const target = new URL(PROVIDER);
    target.pathname = '/' + resource.split('/').map(encodeURIComponent).join('/');
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), 10_000);
    let body;
    try {
      const response = await mapFetch(
        new Request(target, {
          redirect: 'manual',
          credentials: 'omit',
          headers: { Accept: '*/*' },
          signal: controller.signal,
        })
      );
      if (response.status !== 200) {
        await response.body?.cancel();
        throw new Error('Map request failed');
      }
      const contentType = (response.headers.get('Content-Type') ?? '').split(';')[0].trim();
      const json = ['style', 'tilejson', 'sprite'].includes(kind);
      if (
        json
          ? contentType !== 'application/json'
          : kind === 'png'
            ? contentType !== 'image/png'
            : ![
                'application/x-protobuf',
                'application/vnd.mapbox-vector-tile',
                'application/octet-stream',
              ].includes(contentType)
      ) {
        await response.body?.cancel();
        throw new Error('Unexpected map format');
      }
      const bytes = await limitedBody(response, kind === 'style' ? 512 * 1024 : MAX_BYTES);
      if (!bytes.length) throw new Error('Empty map file');
      if (
        kind === 'png' &&
        ![137, 80, 78, 71, 13, 10, 26, 10].every((byte, index) => bytes[index] === byte)
      )
        throw new Error('Invalid map image');
      body = json ? rewriteJson(bytes, kind) : bytes;
    } finally {
      clearTimeout(timeout);
    }
    return new Response(body, {
      headers: {
        'Content-Type': ['style', 'tilejson', 'sprite'].includes(kind)
          ? 'application/json'
          : kind === 'png'
            ? 'image/png'
            : 'application/x-protobuf',
        'Cache-Control': `private, max-age=${['style', 'tilejson'].includes(kind) ? 300 : 86400}`,
        'CDN-Cache-Control': 'no-store',
        'X-Content-Type-Options': 'nosniff',
        'Referrer-Policy': 'no-referrer',
        'Content-Security-Policy': "default-src 'none'; frame-ancestors 'none'",
      },
    });
  } catch {
    return failure(502);
  }
}
