import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { test } from 'node:test';

const script = path.resolve('compose/render-origin-gate.sh');

function render(token, required, inspect) {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'origin-gate-'));
  const output = path.join(directory, 'gate.conf');
  try {
    const result = spawnSync('sh', [script, output], {
      encoding: 'utf8',
      env: { ...process.env, PANDA_GATEWAY_TOKEN: token, PANDA_REQUIRE_GATEWAY: required },
    });
    inspect(result, output, directory);
  } finally {
    fs.rmSync(directory, { recursive: true, force: true });
  }
}

test('required gateway cannot start without its token', () => {
  render('', 'true', (result, output) => {
    assert.notEqual(result.status, 0);
    assert.equal(fs.existsSync(output), false);
  });
});

test('invalid tokens cannot inject nginx directives or leak into errors', () => {
  for (const token of [
    'short',
    'a'.repeat(129),
    `${'a'.repeat(43)}; return 200;`,
    `${'a'.repeat(43)}\n}`,
    `${'a'.repeat(43)}.*`,
    `${'a'.repeat(43)} secret`,
    `${'a'.repeat(43)}é`,
  ]) {
    render(token, 'true', (result, output, directory) => {
      assert.notEqual(result.status, 0);
      assert.equal(fs.existsSync(output), false);
      assert.deepEqual(fs.readdirSync(directory), []);
      assert.equal(`${result.stdout}${result.stderr}`.includes(token), false);
    });
  }
});

test('invalid requirement setting fails closed', () => {
  render('a'.repeat(43), 'tru', (result, output) => {
    assert.notEqual(result.status, 0);
    assert.equal(fs.existsSync(output), false);
  });
});

test('valid token config is private and the token is not printed', () => {
  const token = 'SyntheticGatewayTokenForTestsOnly_0123456789-AbC';
  render(token, 'true', (result, output) => {
    assert.equal(result.status, 0);
    assert.equal(fs.statSync(output).mode & 0o777, 0o600);
    assert.equal(`${result.stdout}${result.stderr}`.includes(token), false);
  });
});

test('private-network deployments can leave the optional gateway unset', () => {
  render('', 'false', (result, output) => {
    assert.equal(result.status, 0);
    assert.equal(fs.statSync(output).mode & 0o777, 0o600);
  });
});
