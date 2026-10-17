import { createRemoteJWKSet, jwtVerify } from 'jose';
import { basemapConfiguration, describeGatewayMaps, mapResource, MAP_PREFIX, MAP_CONFIG, CONNECTIONS } from './maps.mjs';

const keySets = new Map();
const accessSessionSeconds = 7 * 24 * 60 * 60;
const STRIPPED_REQUEST_HEADERS = new Set([
  'cf-access-jwt-assertion', 'cf-access-token', 'cf-access-client-id', 'cf-access-client-secret',
  'cf-access-authenticated-user-email', 'forwarded', 'x-forwarded-for', 'x-forwarded-host',
  'x-forwarded-proto', 'host',
]);
const ENTITY_HEADERS = ['Content-Length', 'Content-Encoding', 'ETag', 'Content-MD5'];

function accessKeys(issuer) {
  const existing = keySets.get(issuer);
  if (existing) return existing;
  const resolver = createRemoteJWKSet(new URL('/cdn-cgi/access/certs', issuer), {
    timeoutDuration: 3000, cacheMaxAge: 600_000, cooldownDuration: 30_000,
  });
  keySets.set(issuer, resolver);
  return resolver;
}

function origin(value) {
  const candidate = new URL(value);
  const forbidden = [candidate.username, candidate.password, candidate.port, candidate.search, candidate.hash];
  if (candidate.protocol === 'https:' && candidate.pathname === '/' && forbidden.every(value => !value)) return candidate.origin;
  throw new Error('Invalid origin configuration');
}

function configuration(env) {
  const [publicOrigin, upstreamOrigin, issuer] = [env.PUBLIC_ORIGIN, env.UPSTREAM_ORIGIN, env.ACCESS_ISSUER].map(origin);
  const valid = publicOrigin !== upstreamOrigin
    && new URL(issuer).hostname.endsWith('.cloudflareaccess.com')
    && /^[A-Za-z0-9_-]{16,128}$/.test(env.ACCESS_AUDIENCE ?? '')
    && /^[A-Za-z0-9_-]{43,128}$/.test(env.GATEWAY_TOKEN ?? '');
  if (!valid) throw new Error('Invalid access configuration');
  const emails = [];
  for (const entry of (env.ALLOWED_EMAILS ?? '').split(',')) {
    const email = entry.trim().toLowerCase();
    if (!email) continue;
    if (!/^[^@\s*]+@[^@\s*]+\.[^@\s*]+$/.test(email)) throw new Error('Explicit email allowlist required');
    emails.push(email);
  }
  if (!emails.length) throw new Error('Explicit email allowlist required');
  return { publicOrigin, upstreamOrigin, issuer, emails };
}

function denied(status, message) {
  const response = new Response(message, { status });
  for (const [name, value] of Object.entries({
    'Content-Type': 'text/plain; charset=utf-8', 'Cache-Control': 'no-store',
    'X-Content-Type-Options': 'nosniff', 'Referrer-Policy': 'no-referrer',
  })) response.headers.set(name, value);
  return response;
}

async function admits(request, incoming, config, env, keySetFor) {
  const assertion = request.headers.get('Cf-Access-Jwt-Assertion');
  if (incoming.origin !== config.publicOrigin || !assertion || assertion.length > 16_384) return false;
  try {
    const verified = await jwtVerify(assertion, keySetFor(config.issuer), {
      issuer: config.issuer, audience: env.ACCESS_AUDIENCE, algorithms: ['RS256'],
      requiredClaims: ['exp', 'iat', 'sub', 'email'], clockTolerance: 5, maxTokenAge: accessSessionSeconds,
    });
    const { email, iat, exp } = verified.payload;
    return exp > iat && exp - iat <= accessSessionSeconds
      && typeof email === 'string' && config.emails.includes(email.toLowerCase());
  } catch { return false; }
}

function forwardRequest(request, incoming, config, token) {
  const headers = new Headers();
  for (const [name, value] of request.headers) {
    if (!STRIPPED_REQUEST_HEADERS.has(name)) headers.append(name, value);
  }
  const cookies = [];
  for (const part of (headers.get('Cookie') ?? '').split(';')) {
    if (!/^CF_Authorization\s*=/.test(part.trim())) cookies.push(part);
  }
  const cookie = cookies.join(';').trim();
  if (cookie) headers.set('Cookie', cookie);
  else headers.delete('Cookie');
  headers.set('X-Panda-Edge', token);
  headers.set('X-Forwarded-Proto', 'https');
  headers.set('X-Forwarded-Host', incoming.host);
  const target = new URL(config.upstreamOrigin);
  target.pathname = incoming.pathname;
  target.search = incoming.search;
  return new Request(target, {
    method: request.method, headers, redirect: 'manual', duplex: 'half',
    body: request.method === 'GET' || request.method === 'HEAD' ? undefined : request.body,
  });
}

async function mapDescription(response, path) {
  const body = await response.json();
  let replacement;
  if (path === MAP_CONFIG) {
    if (typeof body.enabled !== 'boolean') throw new Error('Invalid map configuration');
    replacement = basemapConfiguration();
  } else replacement = describeGatewayMaps(body);
  const headers = new Headers(response.headers);
  for (const name of ENTITY_HEADERS) headers.delete(name);
  return Response.json(replacement, { headers });
}

function privateResponse(response, target, config) {
  const headers = new Headers(response.headers);
  const location = headers.get('Location');
  if (location) {
    const destination = new URL(location, target);
    if (destination.origin !== config.upstreamOrigin) throw new Error('Invalid dashboard redirect');
    const rewritten = new URL(config.publicOrigin);
    for (const component of ['pathname', 'search', 'hash']) rewritten[component] = destination[component];
    headers.set('Location', rewritten.href);
  }
  headers.set('Cache-Control', 'private, no-store');
  headers.set('CDN-Cache-Control', 'no-store');
  headers.delete('X-Panda-Edge');
  return new Response(response.body, { headers, status: response.status, statusText: response.statusText });
}

export function createGateway({ upstreamFetch = fetch, mapFetch = fetch, keySetFor = accessKeys } = {}) {
  async function fetchRequest(request, env) {
    let config;
    try { config = configuration(env); }
    catch { return denied(503, 'Private access is not configured.'); }
    const incoming = new URL(request.url);
    if (!await admits(request, incoming, config, env, keySetFor)) return denied(403, 'Access denied.');
    const managedMaps = env.ENABLE_FREE_MAPS === 'true';
    if (managedMaps) {
      if (request.method === 'POST' && incoming.pathname === `${CONNECTIONS}/disable-optional`) {
        return denied(403, 'Street maps are managed by this deployment.');
      }
      if (incoming.pathname.startsWith(MAP_PREFIX)) return mapResource(request, mapFetch);
    }
    let response;
    let forwarded;
    try {
      forwarded = forwardRequest(request, incoming, config, env.GATEWAY_TOKEN);
      response = await upstreamFetch(forwarded, { cf: { cacheEverything: false, cacheTtl: 0 } });
    } catch { return denied(502, 'Dashboard temporarily unavailable.'); }
    if (response.status === 101) return response;
    const replaceMaps = managedMaps && request.method === 'GET' && response.status === 200
      && (incoming.pathname === MAP_CONFIG || incoming.pathname === CONNECTIONS);
    if (replaceMaps) {
      try { response = await mapDescription(response, incoming.pathname); }
      catch { return denied(502, 'Map configuration unavailable.'); }
    }
    try { return privateResponse(response, forwarded.url, config); }
    catch { return denied(502, 'Unexpected dashboard redirect.'); }
  }
  return { fetch: fetchRequest };
}

export default createGateway();
