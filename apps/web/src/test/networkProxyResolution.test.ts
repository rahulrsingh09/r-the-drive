import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import { resolveApiBaseUrl } from '../../../../packages/hooks/src/api';
import { getWebSocketBaseUrl } from '../../../../packages/hooks/src/useVehicleStatus';

describe('network proxy URL resolution', () => {
  it('proxies both supported versioned REST roots in local and packaged deployments', () => {
    const viteConfig = readFileSync(resolve(process.cwd(), 'vite.config.ts'), 'utf8');
    const nginxConfig = readFileSync(
      resolve(process.cwd(), '../../compose/nginx/nginx.conf'),
      'utf8'
    );

    expect(viteConfig).toMatch(/['"]\/v1['"]\s*:/);
    expect(viteConfig).toMatch(/['"]\/v2['"]\s*:/);
    expect(nginxConfig).toMatch(/location\s+\^~\s+\/v1\//);
    expect(nginxConfig).toMatch(/location\s+\^~\s+\/v2\//);
  });

  it('keeps the SPA shell uncached and only caches existing Vite assets immutably', () => {
    const nginxConfig = readFileSync(
      resolve(process.cwd(), '../../compose/nginx/nginx.conf'),
      'utf8'
    );

    expect(nginxConfig).toMatch(
      /location\s+\^~\s+\/assets\/\s*\{[\s\S]*?try_files\s+\$uri\s+=404;[\s\S]*?expires\s+1y;[\s\S]*?add_header\s+Cache-Control\s+"public, max-age=31536000, immutable";/
    );
    expect(nginxConfig).toMatch(
      /location\s+=\s+\/index\.html\s*\{[\s\S]*?try_files\s+\$uri\s+=404;[\s\S]*?expires\s+off;[\s\S]*?add_header\s+Cache-Control\s+"no-store, no-cache, must-revalidate";/
    );
    expect(nginxConfig).toMatch(
      /location\s+\/\s*\{[\s\S]*?try_files\s+\$uri\s+\$uri\/\s+\/index\.html;[\s\S]*?add_header\s+Cache-Control\s+"no-store, no-cache, must-revalidate";/
    );
    expect(nginxConfig).toMatch(
      /location\s+\^~\s+\/vehicle-images\/fallbacks\/\s*\{[\s\S]*?expires\s+1d;[\s\S]*?add_header\s+Cache-Control\s+"public, max-age=86400";/
    );
    expect(nginxConfig).not.toMatch(/location\s+~\*\s+\\\.\(js\|css\|woff2\?/);
  });

  it('allows the optional browser release check through the SPA content security policy', () => {
    const nginxConfig = readFileSync(
      resolve(process.cwd(), '../../compose/nginx/nginx.conf'),
      'utf8'
    );

    for (const shellLocation of [
      /location\s+=\s+\/index\.html\s*\{([\s\S]*?)\n {8}\}/,
      /location\s+\/\s*\{([\s\S]*?)\n {8}\}/,
    ]) {
      const match = nginxConfig.match(shellLocation);
      expect(match?.[1]).toContain('include /etc/nginx/security-headers.conf;');
      expect(match?.[1]).not.toContain('add_header Content-Security-Policy');
    }
  });

  it('uses one bounded browser policy for maps and optional release checks', () => {
    const headers = readFileSync(
      resolve(process.cwd(), '../../compose/nginx/security-headers.conf'),
      'utf8'
    );
    expect(headers).toContain("connect-src 'self' https://api.github.com;");
    expect(headers).toContain("worker-src 'self' blob:;");
    expect(headers).not.toContain('wss:');
    expect(headers.match(/add_header Content-Security-Policy/g)).toHaveLength(1);
  });

  it('keeps REST calls same-origin for localhost browsers when VITE_API_URL targets localhost', () => {
    const baseUrl = resolveApiBaseUrl('http://localhost:3001', {
      hostname: 'localhost',
      origin: 'http://localhost:5173',
    });

    expect(baseUrl).toBe('');
  });

  it('keeps REST calls same-origin for remote browsers when VITE_API_URL targets localhost', () => {
    const baseUrl = resolveApiBaseUrl('http://localhost:3001', {
      hostname: '192.168.1.25',
      origin: 'http://192.168.1.25:5173',
    });

    expect(baseUrl).toBe('');
  });

  it('keeps websocket calls same-origin for remote browsers when VITE_WS_URL targets localhost', () => {
    const baseUrl = getWebSocketBaseUrl('http://localhost:3001', {
      hostname: '192.168.1.25',
      origin: 'http://192.168.1.25:5173',
    });

    expect(baseUrl).toBe('ws://192.168.1.25:5173');
  });

  it('keeps websocket calls same-origin for localhost browsers when VITE_WS_URL targets localhost', () => {
    const baseUrl = getWebSocketBaseUrl('http://localhost:3001', {
      hostname: 'localhost',
      origin: 'http://localhost:5173',
    });

    expect(baseUrl).toBe('ws://localhost:5173');
  });

  it('preserves explicit non-loopback API targets', () => {
    const baseUrl = resolveApiBaseUrl('http://192.168.1.50:3001', {
      hostname: '192.168.1.25',
      origin: 'http://192.168.1.25:5173',
    });

    expect(baseUrl).toBe('http://192.168.1.50:3001');
  });

  it('supports VITE_PANDA_API_BASE_URL for websocket calls', () => {
    const baseUrl = getWebSocketBaseUrl('http://api.panda.test', {
      hostname: 'panda.test',
      origin: 'https://panda.test',
    });

    expect(baseUrl).toBe('ws://api.panda.test');
  });
});
