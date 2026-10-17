import assert from 'node:assert/strict';
import {execFileSync, spawnSync} from 'node:child_process';
import test from 'node:test';
import {parse} from 'yaml';
import {root, run, step} from './workflow-test-harness.mjs';

test('original health loop reports success or stops after twenty failed probes with an inert child', () => {
  const original = parse(execFileSync('git', ['show', 'dd50456:.github/workflows/runtime.yml'], {cwd: root, encoding: 'utf8'}));
  const program = original.jobs.runtime.steps.find((entry) => entry.name === 'Start API and probe health').run;
  for (const status of [0, 1]) {
    const result = run(program, {rules: [
      {argv: ['cargo', 'run', '--bin', 'panda-api'], hold: true},
      {argv: ['curl', '-sf', 'http://localhost:3001/health'], status, after: ['cargo', 'run', '--bin', 'panda-api']},
      {argv: ['sleep', '2']},
    ]});
    assert.equal(result.status, status, result.stderr);
    assert.equal(result.calls.filter(([command]) => command === 'cargo').length, 1);
    assert.equal(result.calls.filter(([command]) => command === 'curl').length, status === 0 ? 1 : 20);
    assert.equal(result.calls.filter(([command]) => command === 'sleep').length, status === 0 ? 0 : 20);
    assert.equal(result.stdout, status === 0 ? 'Health check passed\n' : 'Health check timed out\n');
  }
});

test('health supervisor bounds each probe, preserves 20 attempts, and cleans its process group', () => {
  const program = step('runtime', 'runtime', 'Start API and probe health');
  const script = program.match(/<<'PY'\n([\s\S]*)\nPY/)[1];
  for (const probes of [[0], [1, 1, 0], Array(20).fill(1), Array(20).fill('timeout'), ['error']]) {
    for (const ignore_term of [false, true]) for (const missing_process of [false, true]) {
      const result = spawnSync('python3', [`${root}/tools/workflow-health-harness.py`], {
        input: JSON.stringify({script, probes, ignore_term, missing_process}), encoding: 'utf8',
        env: {PATH: process.env.PATH, REDIS_URL: 'must-be-overridden'},
      });
      assert.equal(result.status, 0, result.stderr);
      const observed = JSON.parse(result.stdout);
      const healthy = probes.at(-1) === 0;
      assert.equal(observed.status, probes[0] === 'error' ? 'error' : healthy ? 0 : 1);
      assert.deepEqual(observed.events[0], ['spawn', ['cargo', 'run', '--bin', 'panda-api'], {
        env: {...observed.events[0][2].env, REDIS_URL: 'redis://localhost:6379'},
        start_new_session: true,
      }]);
      const calls = observed.events.filter(([kind]) => kind === 'probe');
      assert.equal(calls.length, probes.length);
      for (const call of calls) assert.deepEqual(call, ['probe', ['curl', '-sf', 'http://localhost:3001/health'], {timeout: 2}]);
      assert.deepEqual(observed.events.filter(([kind]) => kind === 'signal'), [
        ['signal', 12345, observed.term], ...(ignore_term ? [['signal', 12345, observed.kill]] : []),
      ]);
      assert.deepEqual(observed.events.filter(([kind]) => kind === 'wait'), [
        ['wait', {timeout: 5}], ...(ignore_term ? [['wait', {}]] : []),
      ]);
      assert.equal(observed.events.filter(([kind]) => kind === 'sleep').length, probes.filter((value) => value === 1 || value === 'timeout').length);
      if (probes[0] !== 'error') assert.equal(observed.stdout, healthy ? 'Health check passed\n' : 'Health check timed out\n');
    }
  }
});
