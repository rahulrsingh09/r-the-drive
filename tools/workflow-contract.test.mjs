import assert from 'node:assert/strict';
import test from 'node:test';
import {digest, run, sha, step} from './workflow-test-harness.mjs';

const head = {argv: ['git', 'rev-parse', 'HEAD'], stdout: `${sha}\n`};
const docker = {prefix: ['docker'], metadata: {'containerimage.descriptor': {digest}}};
const time = {prefix: ['date'], stdout: '1000\n'};
const inspect = {prefix: ['docker', 'buildx', 'imagetools', 'inspect']};

test('patch overlay handles old, already compatible, missing and ambiguous seams', () => {
  const old = 'RUN pnpm install --frozen-lockfile';
  const compatible = 'RUN pnpm config set allowUnusedPatches true --location project';
  for (const job of ['build-amd64', 'build-arm64']) {
    const program = step('publish-candidate-image', job, 'Apply pnpm patch compatibility overlay');
    for (const [source, status, expected] of [
      [`before\n${old}\nafter\n`, 0, `before\n${compatible} \\\n              && pnpm install --frozen-lockfile\nafter\n`],
      [compatible, 0, compatible],
      [`${compatible}\n${old}\n${old}`, 0, `${compatible}\n${old}\n${old}`],
      ['RUN echo absent\n', 1, 'RUN echo absent\n'],
      [`${old}\n${old}`, 1, `${old}\n${old}`],
    ]) {
      const result = run(program, {files: {'compose/Dockerfile': source}});
      assert.equal(result.status, status, result.stderr);
      assert.equal(result.files['compose/Dockerfile'], expected);
    }
  }
});

test('stable and prerelease versions reject malformed input before side effects', () => {
  for (const [name, job, key, valid] of [
    ['publish-release-images', 'validate-tag', 'RELEASE_VERSION', ['2026.01.0', '0000.12.009']],
    ['publish-prerelease-images', 'validate', 'VERSION', ['2026.12.0-rc.0', '2026.01.1-alpha.2', '2026.02.1-beta.002']],
  ]) {
    const program = step(name, job, 'version');
    for (const value of [...valid, '2026.00.0', '2026.13.0', '2026.1.0', 'v2026.01.0', '2026.01.0\n', '$(touch escaped)', '2026.01.0-dev.1']) {
      const result = run(program, {env: {[key]: value, INCLUDE_ARM64: 'false'}, rules: [
        head, {argv: ['git', 'fetch', 'origin', 'main']},
        {argv: ['git', 'merge-base', '--is-ancestor', sha, 'origin/main']},
      ]});
      assert.equal(result.status, valid.includes(value) ? 0 : 1, `${name}: ${value}\n${result.stderr}`);
      if (valid.includes(value)) assert.equal(result.output, `version=${value}\nsource_sha=${sha}\ninclude_arm64=false\n`);
      else assert.deepEqual(result.calls, []);
    }
  }
});

test('stable release ancestry denial stops output and promotion', () => {
  const result = run(step('publish-release-images', 'validate-tag', 'version'), {
    env: {RELEASE_VERSION: '2026.10.0', INCLUDE_ARM64: 'true'},
    rules: [{argv: ['git', 'fetch', 'origin', 'main']},
      {argv: ['git', 'merge-base', '--is-ancestor', sha, 'origin/main'], status: 1}],
  });
  assert.equal(result.status, 1);
  assert.equal(result.output, '');
});

test('promotion chooses exact platform candidates and preserves release stamp and tag policy', () => {
  for (const stable of [false, true]) for (const arm of ['false', 'true']) {
    const result = run(step(stable ? 'publish-release-images' : 'publish-prerelease-images', 'promote-image', 'promote'), {
      env: {VERSION: '2026.10.0', INCLUDE_ARM64: arm},
      rules: [time, docker], files: {'compose/Dockerfile.release-stamp': 'fixture stamp\n'},
    });
    assert.equal(result.status, 0, result.stderr);
    assert.equal(result.output, `digest=${digest}\n`);
    const arches = arm === 'true' ? ['amd64', 'arm64'] : ['amd64'];
    assert.deepEqual(result.calls.filter((args) => args[3] === 'inspect'), arches.map((arch) => [
      'docker', 'buildx', 'imagetools', 'inspect', `registry.invalid/fixture:candidate-${sha}-${arch}`,
    ]));
    const create = result.calls.find((args) => args[3] === 'create');
    assert.deepEqual(create.slice(6), [
      '--tag', 'registry.invalid/fixture:2026.10.0',
      ...(stable ? ['--tag', 'registry.invalid/fixture:latest'] : []),
      ...arches.map((arch) => `registry.invalid/fixture:${stable ? 'release-2026.10.0' : `candidate-${sha}`}-${arch}`),
    ]);
    const builds = result.calls.filter((args) => args[2] === 'build');
    assert.equal(builds.length, stable ? arches.length : 0);
    for (const [index, args] of builds.entries()) assert.deepEqual(args, [
      'docker', 'buildx', 'build', '--platform', `linux/${arches[index]}`, '--provenance=false', '--push',
      '--build-arg', `BASE_IMAGE=registry.invalid/fixture:candidate-${sha}-${arches[index]}`,
      '--build-arg', 'PANDA_BUILD_VERSION=2026.10.0', '--tag', `registry.invalid/fixture:release-2026.10.0-${arches[index]}`, '-',
    ]);
  }
});

test('promotion fails closed for missing candidates, invalid metadata and command failure', () => {
  for (const name of ['publish-release-images', 'publish-prerelease-images']) {
    const program = step(name, 'promote-image', 'promote');
    for (const rules of [
      [time, {...inspect, status: 1}],
      [time, {...docker, status: 9}],
      ...[{}, {'containerimage.digest': 'latest'}, {'containerimage.digest': `sha256:${'A'.repeat(64)}`},
        {'containerimage.descriptor': {digest: ''}, 'containerimage.digest': digest},
      ].map((metadata) => [time, {...docker, metadata}]),
    ]) {
      const result = run(program, {env: {VERSION: '2026.10.0', INCLUDE_ARM64: 'false'}, rules, files: {'compose/Dockerfile.release-stamp': 'fixture\n'}});
      assert.notEqual(result.status, 0);
      assert.equal(result.output, '');
    }
    const fallback = run(program, {env: {VERSION: '2026.10.0', INCLUDE_ARM64: 'false'}, rules: [
      time, {...docker, metadata: {'containerimage.descriptor': {digest: null}, 'containerimage.digest': digest}},
    ], files: {'compose/Dockerfile.release-stamp': 'fixture\n'}});
    assert.equal(fallback.status, 0, fallback.stderr);
    assert.equal(fallback.output, `digest=${digest}\n`);
  }
});

test('dev promotion verifies source ref, current SHA and published digest before reporting success', () => {
  const program = step('publish-candidate-image', 'promote-dev', 'promote');
  for (const [ref, current, published, expected] of [
    ['main', sha, digest, false], ['dev', 'c'.repeat(40), digest, false],
    ['dev', sha, `sha256:${'c'.repeat(64)}`, false], ['dev', sha, digest, true],
  ]) for (const platform of ['amd64', 'both']) {
    const result = run(program, {
      env: {SOURCE_REF: ref, PLATFORM: platform}, rules: [
        {prefix: ['gh', 'api'], stdout: `${current}\n`},
        {argv: ['docker', 'buildx', 'imagetools', 'inspect', 'registry.invalid/fixture:dev', '--format', '{{.Manifest.Digest}}'], stdout: `${published}\n`},
        docker,
      ],
    });
    assert.equal(result.status, expected ? 0 : 1, result.stderr);
    assert.equal(result.output, expected ? `digest=${digest}\n` : '');
    if (ref !== 'dev') assert.deepEqual(result.calls, []);
    else if (current !== sha) assert.equal(result.calls.length, 1);
  }
});

test('retention preserves moving, shared and unrelated tags and keeps 10 AMD64 / 3 ARM64', () => {
  const rows = [];
  for (const arch of ['amd64', 'arm64']) for (let i = 1; i <= 14; i++) {
    rows.push(`2026-10-${String(i).padStart(2, '0')}T00:00:00Z\t${arch === 'amd64' ? 100 + i : 200 + i}\tcandidate-${sha}-${arch}`);
  }
  rows.push(...['dev', 'latest', `candidate-${sha}-amd64,latest`, `candidate-${sha.toUpperCase()}-arm64`, 'buildcache-amd64-v2', '2026.10.0', 'abcdef0-dev,dev'].map((tag, i) => `2026-01-01\t${900 + i}\t${tag}`));
  rows.push('2020-01-01\t999\tabcdef0-dev');
  const result = run(step('publish-candidate-image', 'prune-candidates', 'Retain recent commit candidates'), {
    rules: [
      {prefix: ['gh', 'api', '--paginate'], stdout: rows.reverse().join('\n') + '\n'},
      {prefix: ['gh', 'api', '--method', 'DELETE']},
    ],
  });
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(result.calls.slice(1).map((args) => Number(args.at(-1).split('/').at(-1))), [
    104, 103, 102, 101, 211, 210, 209, 208, 207, 206, 205, 204, 203, 202, 201, 999,
  ]);
  assert.equal(result.summary, '- Stale candidates deleted: 16\n');
});

test('retention list and deletion failures cannot report successful cleanup', () => {
  const program = step('publish-candidate-image', 'prune-candidates', 'Retain recent commit candidates');
  for (const rules of [
    [{prefix: ['gh', 'api', '--paginate'], status: 1}],
    [{prefix: ['gh', 'api', '--paginate'], stdout: '2020-01-01\t12\tabcdef0-dev\n'}, {prefix: ['gh', 'api', '--method', 'DELETE'], status: 1}],
  ]) {
    const result = run(program, {rules});
    assert.notEqual(result.status, 0);
    assert.equal(result.summary, '');
  }
});

test('smoke environment transforms only assigned keys and appends random setup token', () => {
  for (const name of ['release', 'prerelease']) {
    const source = '# fixture\nPOSTGRES_PASSWORD=old\nREDIS_PASSWORD=old\nALLOWED_ORIGINS=old\nOTHER=keep\n';
    const result = run(step(`publish-${name}-images`, 'release-smoke', 'Create ephemeral production environment'), {
      files: {'compose/.env.example': source},
      rules: [{argv: ['openssl', 'rand', '-hex', '32'], stdout: 'c'.repeat(64) + '\n'}],
    });
    assert.equal(result.status, 0, result.stderr);
    assert.equal(result.files['compose/.env.example'], source);
    assert.match(result.smoke, new RegExp(`POSTGRES_PASSWORD=${name}-smoke-password\\nREDIS_PASSWORD=${name}-smoke-redis-password\\nALLOWED_ORIGINS=https://${name}-smoke.example.test\\nOTHER=keep\\nPANDA_SETUP_TOKEN=[0-9a-f]{64}\\n$`));
    assert.match(result.environment, new RegExp(`^${name.toUpperCase()}_SMOKE_ENV=.+/${name}-smoke.env\\n$`));
  }
});

test('release packaging targets the exact source and stable existing release gets asset replacement', () => {
  for (const stable of [true, false]) for (const existing of [true, false]) {
    const result = run(step(stable ? 'publish-release-images' : 'publish-prerelease-images', 'create-release',
      stable ? 'Create digest lock and GitHub release' : 'Create digest lock and GitHub pre-release'), {
      env: {VERSION: '2026.10.0', IMAGE_DIGEST: digest},
      files: {'compose/docker-compose.yml': 'compose fixture\n', 'compose/.env.example': 'env fixture\n'},
      rules: [{prefix: ['gh', 'release', 'view'], status: existing ? 0 : 1}, {prefix: ['gh', 'release']}],
    });
    assert.equal(result.status, 0, result.stderr);
    assert.equal(result.lock, `ghcr.io/your-org/r-the-drive@${digest}\n`);
    const command = result.calls.at(-1);
    assert.equal(command[2], stable && existing ? 'upload' : 'create');
    assert.equal(command.includes('--clobber'), stable && existing);
    assert.equal(command.includes('--prerelease'), !stable);
    if (!stable) assert.equal(command[command.indexOf('--target') + 1], sha);
  }
});

test('hygiene accepts normal paths and rejects each generated-state segment or file', () => {
  const program = step('quality', 'quality', 'Reject generated state and duplicate lockfiles');
  for (const name of ['a/.codex/settings', '.claude/x', 'node_modules/a', 'a/target/x', 'dist/a', 'coverage/x', '.turbo/x', '.playwright-cli/x', 'package-lock.json', 'a/test_output.txt', 'x/a.tmp']) {
    const result = run(program, {rules: [{prefix: ['git', 'ls-files'], stdout: `src/good.ts\n${name}\n`}]});
    assert.equal(result.status, 1, name);
    assert.match(result.stdout, /Generated or local-only/);
  }
  const result = run(program, {rules: [{prefix: ['git', 'ls-files'], stdout: 'src/target.ts\npackage.json\nfoo.tmp/file\nsrc/distribution.ts\n'}]});
  assert.equal(result.status, 0);
});

test('path gates run conservatively on diff errors and skip only successful empty comparisons', () => {
  for (const [name, job, title, key] of [
    ['quality', 'quality', 'Detect documentation-site changes', 'changed'],
    ['restore-compatibility', 'contracts', 'Detect restore-contract changes', 'run'],
    ['vehicle-artwork-build', 'validate', 'Detect artwork-contract changes', 'run'],
  ]) {
    const program = step(name, job, title);
    for (const status of [0, 1, 128]) {
      const result = run(program, {env: {BASE_SHA: sha}, substitutions: {'github.event_name': 'pull_request'}, rules: [
        {prefix: ['git', 'diff'], status, stdout: status === 1 ? 'docs/index.md\n' : ''},
      ]});
      assert.equal(result.status, 0, result.stderr);
      assert.equal(result.output, `${key}=${status !== 0}\n`);
    }
    if (key === 'run') {
      const result = run(program, {env: {BASE_SHA: ''}, substitutions: {'github.event_name': 'workflow_dispatch'}});
      assert.equal(result.status, 0);
      assert.equal(result.output, 'run=true\n');
      assert.deepEqual(result.calls, []);
    }
  }
});

test('readiness distinguishes failure from a skipped promotion while preserving exact candidate', () => {
  const program = step('fork-upstream-sync', 'readiness', 'Report readiness for the exact candidate');
  for (const sync of ['success', 'failure', 'skipped']) for (const automation of ['success', 'failure', 'skipped']) {
    const result = run(program, {env: {CANDIDATE: sha, SYNC_STATUS: '', SYNC_RESULT: sync, AUTOMATION_RESULT: automation}});
    assert.equal(result.status, sync !== 'success' || automation === 'failure' ? 1 : 0);
    const data = JSON.parse(result.summary.split('```json\n')[1].split('\n```')[0]);
    assert.deepEqual(data, {candidate: sha, sync, automation, needs_review: !(sync === 'success' && automation === 'success')});
  }
});
