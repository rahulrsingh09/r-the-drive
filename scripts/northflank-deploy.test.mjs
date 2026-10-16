import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import test from 'node:test';

test('local Northflank controller: synthetic Python adapter regressions', () => {
  const result = spawnSync('python3', ['-B', '-m', 'unittest', 'discover', '-s', 'scripts',
    '-p', 'test_northflank_deploy.py', '-v'], {
    cwd: new URL('..', import.meta.url),
    encoding: 'utf8',
    timeout: 30_000,
  });
  assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
});
