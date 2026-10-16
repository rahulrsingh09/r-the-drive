#!/usr/bin/env node
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile, stat } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { dirname, extname, join, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const dist = resolve(process.argv[2] ?? join(root, 'apps/web/dist'));
const require = createRequire(join(root, 'apps/web/package.json'));
const { chromium, expect } = require('@playwright/test');
const nginx = await readFile(join(root, 'compose/nginx/security-headers.conf'), 'utf8');
const policy = nginx.match(/add_header Content-Security-Policy "([^"]+)" always;/)?.[1];
assert.ok(policy && !policy.includes("'unsafe-eval'"), 'The production script policy must forbid eval.');
const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.svg': 'image/svg+xml' };
const server = createServer(async (request, response) => {
  response.setHeader('Content-Security-Policy', policy);
  const pathname = new URL(request.url, 'http://localhost').pathname;
  if (pathname.startsWith('/v1/')) {
    response.writeHead(503, { 'content-type': 'application/json' });
    response.end(JSON.stringify({ error: 'API is intentionally unavailable in this static browser check.' }));
    return;
  }
  let target = resolve(dist, '.' + decodeURIComponent(pathname));
  if (!target.startsWith(dist + sep) && target !== dist) {
    response.writeHead(404); response.end(); return;
  }
  try {
    if (!(await stat(target)).isFile()) target = join(dist, 'index.html');
  } catch { target = join(dist, 'index.html'); }
  try {
    response.writeHead(200, { 'content-type': types[extname(target)] ?? 'application/octet-stream' });
    response.end(await readFile(target));
  } catch {
    response.writeHead(404); response.end();
  }
});
await new Promise((ready) => server.listen(0, '127.0.0.1', ready));
const origin = `http://127.0.0.1:${server.address().port}`;
let browser;
try {
  browser = await chromium.launch({ headless: true });
  const context = await browser.newContext();
  const violations = [];
  await context.exposeBinding('recordPolicyViolation', (_, value) => violations.push(value));
  await context.addInitScript(() => {
    document.addEventListener('securitypolicyviolation', (event) => {
      window.recordPolicyViolation({ directive: event.effectiveDirective,
        blockedURI: event.blockedURI, source: event.sourceFile, line: event.lineNumber });
    });
  });
  await context.route('**/*', (route) => new URL(route.request().url()).origin === origin
    ? route.continue() : route.abort('blockedbyclient'));
  const page = await context.newPage();
  await page.goto(origin + '/login');
  await expect(page.getByText('R:The Drive', { exact: true })).toBeVisible();
  await page.evaluate(() => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))));
  assert.deepEqual(violations, [], 'Production browser startup violated its script policy.');
  console.log('Production browser bootstrap passed strict CSP without dynamic code.');
} finally {
  if (browser) await browser.close();
  server.closeAllConnections();
  await new Promise((done) => server.close(done));
}
