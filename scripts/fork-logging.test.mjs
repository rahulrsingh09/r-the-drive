import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';

test('proxy logs omit capability-bearing request fields', () => {
  const config = readFileSync(new URL('../compose/nginx/nginx.conf', import.meta.url), 'utf8');
  const log = config.match(/log_format logfmt[\s\S]*?;/)?.[0];
  assert.ok(log);
  for (const field of [
    '$request_uri',
    '$request',
    '$args',
    '$query_string',
    '$uri',
    '$http_referer',
    '$http_authorization',
    '$http_cookie',
    '$http_sec_websocket_protocol',
  ]) {
    const variable = new RegExp(`${field.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\b`);
    assert.ok(!variable.test(log), `Unsafe access-log field: ${field}`);
  }
  assert.match(config, /error_log\s+\/dev\/null\s+crit;/);
  assert.match(config, /~\^\/v1\/ "\/v1\/\[redacted\]";/);
});
