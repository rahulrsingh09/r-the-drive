import { createRemoteJWKSet, jwtVerify } from 'jose';
import {
  basemapConfiguration,
  describeGatewayMaps,
  mapResource,
  MAP_PREFIX,
  MAP_CONFIG,
  CONNECTIONS,
} from './maps.mjs';

const keySets = new Map();
const accessSessionSeconds = 7 * 24 * 60 * 60;

function accessKeys(issuer) {
  if (!keySets.has(issuer)) {
    keySets.set(
      issuer,
      createRemoteJWKSet(new URL('/cdn-cgi/access/certs', issuer), {
        timeoutDuration: 3000,
        cacheMaxAge: 600_000,
        cooldownDuration: 30_000,
      })
    );
  }
  return keySets.get(issuer);
}

function origin(value) {
  const url = new URL(value);
  if (
    url.protocol !== 'https:' ||
    url.username ||
    url.password ||
    url.port ||
    url.pathname !== '/' ||
    url.search ||
    url.hash
  ) {
    throw new Error('Invalid origin configuration');
  }
  return url.origin;
}

function configuration(env) {
  const publicOrigin = origin(env.PUBLIC_ORIGIN);
  const upstreamOrigin = origin(env.UPSTREAM_ORIGIN);
  const issuer = origin(env.ACCESS_ISSUER);
  if (
    publicOrigin === upstreamOrigin ||
    !new URL(issuer).hostname.endsWith('.cloudflareaccess.com') ||
    !/^[A-Za-z0-9_-]{16,128}$/.test(env.ACCESS_AUDIENCE ?? '') ||
    !/^[A-Za-z0-9_-]{43,128}$/.test(env.GATEWAY_TOKEN ?? '')
  ) {
    throw new Error('Invalid access configuration');
  }
  const emails = (env.ALLOWED_EMAILS ?? '')
    .split(',')
    .map((email) => email.trim().toLowerCase())
    .filter(Boolean);
  if (!emails.length || emails.some((email) => !/^[^@\s*]+@[^@\s*]+\.[^@\s*]+$/.test(email))) {
    throw new Error('Explicit email allowlist required');
  }
  return { publicOrigin, upstreamOrigin, issuer, emails };
}

function denied(status, message) {
  return new Response(message, {
    status,
    headers: {
      'Content-Type': 'text/plain; charset=utf-8',
      'Cache-Control': 'no-store',
      'X-Content-Type-Options': 'nosniff',
      'Referrer-Policy': 'no-referrer',
    },
  });
}

export function createGateway({
  upstreamFetch = fetch,
  mapFetch = fetch,
  keySetFor = accessKeys,
} = {}) {
  return {
    async fetch(request, env) {
      let config;
      try {
        config = configuration(env);
      } catch {
        return denied(503, 'Private access is not configured.');
      }

      const incoming = new URL(request.url);
      if (incoming.origin !== config.publicOrigin) {
        return denied(403, 'Access denied.');
      }
      const assertion = request.headers.get('Cf-Access-Jwt-Assertion');
      if (!assertion || assertion.length > 16_384) {
        return denied(403, 'Access denied.');
      }
      try {
        const { payload } = await jwtVerify(assertion, keySetFor(config.issuer), {
          issuer: config.issuer,
          audience: env.ACCESS_AUDIENCE,
          algorithms: ['RS256'],
          requiredClaims: ['exp', 'iat', 'sub', 'email'],
          clockTolerance: 5,
          maxTokenAge: accessSessionSeconds,
        });
        if (
          payload.exp <= payload.iat ||
          payload.exp - payload.iat > accessSessionSeconds ||
          typeof payload.email !== 'string' ||
          !config.emails.includes(payload.email.toLowerCase())
        ) {
          return denied(403, 'Access denied.');
        }
      } catch {
        return denied(403, 'Access denied.');
      }

      const mapsEnabled = env.ENABLE_FREE_MAPS === 'true';
      if (
        mapsEnabled &&
        request.method === 'POST' &&
        incoming.pathname === CONNECTIONS + '/disable-optional'
      ) {
        return denied(403, 'Street maps are managed by this deployment.');
      }
      if (mapsEnabled && incoming.pathname.startsWith(MAP_PREFIX)) {
        return mapResource(request, mapFetch);
      }

      const target = new URL(config.upstreamOrigin);
      target.pathname = incoming.pathname;
      target.search = incoming.search;
      const headers = new Headers(request.headers);
      for (const name of [
        'Cf-Access-Jwt-Assertion',
        'Cf-Access-Token',
        'Cf-Access-Client-Id',
        'Cf-Access-Client-Secret',
        'Cf-Access-Authenticated-User-Email',
        'Forwarded',
        'X-Forwarded-For',
        'X-Forwarded-Host',
        'X-Forwarded-Proto',
        'Host',
      ]) {
        headers.delete(name);
      }
      const cookies = (headers.get('Cookie') ?? '')
        .split(';')
        .filter((cookie) => !/^CF_Authorization\s*=/.test(cookie.trim()))
        .join(';')
        .trim();
      if (cookies) headers.set('Cookie', cookies);
      else headers.delete('Cookie');
      headers.set('X-Panda-Edge', env.GATEWAY_TOKEN);
      headers.set('X-Forwarded-Proto', 'https');
      headers.set('X-Forwarded-Host', incoming.host);

      let response;
      try {
        response = await upstreamFetch(
          new Request(target, {
            method: request.method,
            headers,
            body: ['GET', 'HEAD'].includes(request.method) ? undefined : request.body,
            redirect: 'manual',
            duplex: 'half',
          }),
          { cf: { cacheEverything: false, cacheTtl: 0 } }
        );
      } catch {
        return denied(502, 'Dashboard temporarily unavailable.');
      }
      // Preserve the WebSocket object on an upgrade response.
      if (response.status === 101) return response;

      if (
        mapsEnabled &&
        request.method === 'GET' &&
        response.status === 200 &&
        [MAP_CONFIG, CONNECTIONS].includes(incoming.pathname)
      ) {
        try {
          const body = await response.json();
          if (incoming.pathname === MAP_CONFIG && typeof body.enabled !== 'boolean') {
            return denied(502, 'Map configuration unavailable.');
          }
          const headers = new Headers(response.headers);
          for (const name of ['Content-Length', 'Content-Encoding', 'ETag', 'Content-MD5']) {
            headers.delete(name);
          }
          response = Response.json(
            incoming.pathname === MAP_CONFIG ? basemapConfiguration() : describeGatewayMaps(body),
            { headers }
          );
        } catch {
          return denied(502, 'Map configuration unavailable.');
        }
      }

      const responseHeaders = new Headers(response.headers);
      responseHeaders.set('Cache-Control', 'private, no-store');
      responseHeaders.set('CDN-Cache-Control', 'no-store');
      responseHeaders.delete('X-Panda-Edge');
      const location = responseHeaders.get('Location');
      if (location) {
        let destination;
        try {
          destination = new URL(location, target);
        } catch {
          return denied(502, 'Unexpected dashboard redirect.');
        }
        if (destination.origin !== config.upstreamOrigin) {
          return denied(502, 'Unexpected dashboard redirect.');
        }
        const rewritten = new URL(config.publicOrigin);
        rewritten.pathname = destination.pathname;
        rewritten.search = destination.search;
        rewritten.hash = destination.hash;
        responseHeaders.set('Location', rewritten.href);
      }
      return new Response(response.body, {
        status: response.status,
        statusText: response.statusText,
        headers: responseHeaders,
      });
    },
  };
}

export default createGateway();
