import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {readFileSync} from 'node:fs';
import test from 'node:test';
import {root, run} from './workflow-test-harness.mjs';

test('API image compiles copied sources once with a locked and bounded build', () => {
  const source = readFileSync(`${root}/apps/api/Dockerfile`, 'utf8');
  const builder = source.split(' AS runtime')[0].replace(/\\\n/g, ' ');
  const commands = [...builder.matchAll(/^RUN (.*)$/gm)].map((match) => match[1]);
  const compile = commands.filter((command) => command.includes('cargo build'));
  assert.deepEqual(compile, ['cargo build --locked --release --jobs 2']);
  assert.ok(source.indexOf('COPY . .') < source.indexOf(`RUN ${compile[0]}`));
  const files = {'src/lib.rs': 'real library\n', 'src/main.rs': 'real entrypoint\n'};
  for (const status of [0, 17]) {
    const result = run(compile[0], {files, rules: [
      {argv: ['cargo', 'build', '--locked', '--release', '--jobs', '2'], status},
    ]});
    assert.equal(result.status, status);
    assert.deepEqual(result.files, files);
  }
});

test('API image retains exact base pins, offline compilation and runtime security recipe', () => {
  const source = readFileSync(`${root}/apps/api/Dockerfile`, 'utf8');
  const baseline = execFileSync('git', ['show', 'dd50456:apps/api/Dockerfile'], {cwd: root, encoding: 'utf8'});
  assert.equal(source.slice(source.indexOf('# ── Runtime stage')), baseline.slice(baseline.indexOf('# ── Runtime stage')));
  assert.deepEqual(source.match(/^FROM .*$/gm), baseline.match(/^FROM .*$/gm));
  assert.match(source, /^ENV SQLX_OFFLINE=true$/m);
});
