import assert from 'node:assert/strict';
import {readdirSync} from 'node:fs';
import {spawnSync} from 'node:child_process';
import test from 'node:test';
import {root, run, sha, step, workflow} from './workflow-test-harness.mjs';

test('migration baseline accepts only nonzero lowercase SHA and falls back parent then HEAD', () => {
  const program = step('fork-ci', 'frontend', 'Verify deployed migration identities');
  for (const supplied of [sha, '0'.repeat(40), '', 'main', sha.toUpperCase()]) {
    for (const hasParent of [true, false]) {
      const parent = 'c'.repeat(40);
      const expected = supplied === sha ? sha : hasParent ? parent : sha;
      const result = run(program, {env: {BASE_SHA: supplied}, rules: [
        {argv: ['git', 'rev-parse', 'HEAD^1'], stdout: hasParent ? `${parent}\n` : '', status: hasParent ? 0 : 1},
        {argv: ['git', 'rev-parse', 'HEAD'], stdout: `${sha}\n`},
        {argv: ['node', 'scripts/fork-migration-integrity.mjs', expected]},
      ]});
      assert.equal(result.status, 0, result.stderr);
      assert.deepEqual(result.calls.at(-1), ['node', 'scripts/fork-migration-integrity.mjs', expected]);
      assert.equal(result.calls.length, supplied === sha ? 1 : hasParent ? 2 : 3);
    }
  }
  const unavailable = run(program, {env: {BASE_SHA: ''}, rules: [
    {prefix: ['git', 'rev-parse'], status: 1},
  ]});
  assert.notEqual(unavailable.status, 0);
  assert.equal(unavailable.calls.some(([command]) => command === 'node'), false);
  const rejected = run(program, {env: {BASE_SHA: sha}, rules: [
    {argv: ['node', 'scripts/fork-migration-integrity.mjs', sha], status: 17},
  ]});
  assert.notEqual(rejected.status, 0);
});

test('audit runners always pass report bytes and failure status through the existing evaluator', () => {
  for (const [name, job, title, command, report, checker] of [
    ['fork-ci', 'frontend', 'Evaluate every npm advisory', 'pnpm', 'npm', 'scripts/check-dependency-audits.mjs'],
    ['fork-ci', 'backend', 'Evaluate every Cargo advisory', 'cargo', 'cargo', '../../scripts/check-dependency-audits.mjs'],
    ['security', 'security', 'Cargo audit', 'cargo', 'cargo', '../../scripts/check-dependency-audits.mjs'],
    ['security', 'security', 'pnpm audit', 'pnpm', 'pnpm', 'scripts/check-dependency-audits.mjs'],
  ]) for (const status of [0, 1, 42]) for (const verdict of [0, 9]) {
    const result = run(step(name, job, title), {rules: [
      {prefix: ['cargo', 'install']},
      {prefix: [command, 'audit'], stdout: '{"advisories":[]}\n', status},
      {prefix: ['node', checker, command], status: verdict},
    ]});
    assert.equal(result.status === 0, verdict === 0, result.stderr);
    assert.equal(result.reports[report], '{"advisories":[]}\n');
    assert.equal(result.calls.at(-1).at(-1), String(status));
    assert.match(result.calls.at(-1).at(-2), new RegExp(`/${report}-audit.json$`));
    if (name === 'security') assert.equal(result.stdout, '{"advisories":[]}\n');
    if (name === 'fork-ci' && command === 'cargo') {
      assert.deepEqual(result.calls[0], ['cargo', 'install', 'cargo-audit', '--version', '0.22.2', '--locked']);
    }
  }
});

test('release preparation delegates version selection then stops at every failed command', () => {
  const program = step('prepare-release', 'create-tag', 'Calculate and push the next Calendar Version');
  const month = new Date().toISOString().slice(0, 7).replace('-', '.');
  const version = `${month}.3`;
  const commands = [
    ['git', 'tag', '--list', `${month}.*`],
    ['node', 'scripts/next-release-version.mjs', '--month', month, `${month}.1`, `${month}.2`],
    ['git', 'config', 'user.name', 'github-actions[bot]'],
    ['git', 'config', 'user.email', '41898282+github-actions[bot]@users.noreply.github.com'],
    ['git', 'tag', '-a', version, '-m', `Release ${version}`],
    ['git', 'push', 'origin', version],
  ];
  for (const failAt of [-1, 1, 2, 3, 4, 5]) {
    const rules = commands.map((argv, index) => ({
      argv, status: index === failAt ? 23 : 0,
      stdout: index === 0 ? `${month}.1\n${month}.2\n` : index === 1 ? `${version}\n` : '',
    }));
    const result = run(program, {rules: [
      {argv: ['date', '-u', '+%Y.%m'], stdout: `${month}\n`}, ...rules,
    ]});
    assert.equal(result.status === 0, failAt === -1, result.stderr);
    assert.deepEqual(result.calls.filter(([command]) => command !== 'date'),
      commands.slice(0, failAt === -1 ? commands.length : failAt + 1));
    assert.equal(result.output, failAt === 1 ? '' : `version=${version}\n`);
  }
});

test('audit signal termination is passed to the evaluator as a shell-compatible status', () => {
  const result = run(step('fork-ci', 'frontend', 'Evaluate every npm advisory'), {rules: [
    {prefix: ['pnpm', 'audit'], stdout: '{"incomplete":true}\n', signal: 'SIGTERM'},
    {prefix: ['node', 'scripts/check-dependency-audits.mjs']},
  ]});
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.calls.at(-1).at(-1), '143');
  assert.equal(result.reports.npm, '{"incomplete":true}\n');
});

test('path gates request validation when Git successfully reports changed paths', () => {
  for (const [name, job, title, key] of [
    ['quality', 'quality', 'Detect documentation-site changes', 'changed'],
    ['restore-compatibility', 'contracts', 'Detect restore-contract changes', 'run'],
    ['vehicle-artwork-build', 'validate', 'Detect artwork-contract changes', 'run'],
  ]) {
    const program = step(name, job, title);
    const result = run(program, {
      env: {BASE_SHA: sha}, substitutions: {'github.event_name': 'pull_request'},
      rules: [{prefix: ['git', 'diff'], status: program.includes('--quiet') ? 1 : 0, stdout: 'changed path\\0'}],
    });
    assert.equal(result.status, 0, result.stderr);
    assert.equal(result.output, `${key}=true\n`);
  }
});

test('all environment transformations preserve comments, CRLF, duplicates and missing final newline', () => {
  const source = '# fixture\r\nOTHER=keep\r\n# POSTGRES_PASSWORD=old\r\nPOSTGRES_PASSWORD=again\r\nDATABASE_URL=old\r\nREDIS_PASSWORD=old\r\nALLOWED_ORIGINS=old';
  for (const kind of ['fresh-install', 'release', 'prerelease']) {
    const fresh = kind === 'fresh-install';
    const result = run(step(fresh ? kind : `publish-${kind}-images`, fresh ? kind : 'release-smoke',
      fresh ? 'Run isolated acceptance' : 'Create ephemeral production environment'), {
      files: {'compose/.env.example': source},
      substitutions: {"inputs.mode || 'production'": 'production'},
      rules: [
        {argv: ['openssl', 'rand', '-hex', '32'], stdout: 'a'.repeat(64) + '\n'},
        {prefix: ['pnpm', 'verify:fresh-install']},
      ],
    });
    assert.equal(result.status, 0, result.stderr);
    const prefix = fresh ? kind : `${kind}-smoke`;
    let expected = source
      .replace(/POSTGRES_PASSWORD=.*/g, `POSTGRES_PASSWORD=${prefix}-password`)
      .replace(/REDIS_PASSWORD=.*/g, `REDIS_PASSWORD=${prefix}-redis-password`)
      .replace(/ALLOWED_ORIGINS=.*/g, `ALLOWED_ORIGINS=https://${prefix}.example.test`);
    // sed's dot consumes CR; JavaScript's dot excludes it.
    expected = expected.replace(/((?:POSTGRES_PASSWORD|REDIS_PASSWORD|ALLOWED_ORIGINS)=[^\r\n]*)\r/g, '$1');
    if (fresh) expected = expected.replace(/DATABASE_URL=[^\n]*/g, 'DATABASE_URL=postgresql://panda:fresh-install-password@timescaledb:5432/panda');
    assert.match(result.smoke, /PANDA_SETUP_TOKEN=[0-9a-f]{64}\n$/);
    assert.equal(result.smoke.replace(/PANDA_SETUP_TOKEN=[0-9a-f]{64}\n$/, ''), expected);
    assert.equal(result.files['compose/.env.example'], source);
    if (fresh) {
      assert.deepEqual(result.calls.at(-1).slice(0, 5), ['pnpm', 'verify:fresh-install', '--', '--mode', 'production']);
      assert.equal(result.calls.at(-1).at(-1), '--source-build');
    }
  }
});

test('workflow shell and embedded Python programs compile without executing commands', () => {
  for (const file of readdirSync(`${root}/.github/workflows`).filter((file) => file.endsWith('.yml'))) {
    const document = workflow(file.replace(/\.yml$/, ''));
    for (const job of Object.values(document.jobs)) for (const [index, entry] of (job.steps ?? []).entries()) {
      if (!entry.run) continue;
      const shell = entry.shell ?? job.defaults?.run?.shell ?? (String(job['runs-on']).includes('windows') ? 'pwsh' : 'bash');
      if (shell === 'bash') {
        const result = spawnSync('/bin/bash', ['-n'], {input: entry.run, encoding: 'utf8'});
        assert.equal(result.status, 0, `${file}/${index}: ${result.stderr}`);
      }
      for (const match of entry.run.matchAll(/python3 -[^\n]* <<'PY'\n([\s\S]*?)^PY$/gm)) {
        const result = spawnSync('python3', ['-c', 'import ast,sys; ast.parse(sys.stdin.read())'], {input: match[1], encoding: 'utf8'});
        assert.equal(result.status, 0, `${file}/${index}: ${result.stderr}`);
      }
    }
  }
});
