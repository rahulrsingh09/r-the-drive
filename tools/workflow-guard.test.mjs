import assert from 'node:assert/strict';
import {copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync} from 'node:fs';
import {spawnSync} from 'node:child_process';
import {tmpdir} from 'node:os';
import path from 'node:path';
import test from 'node:test';
import {root} from './workflow-test-harness.mjs';

test('proposed root checker patch accepts replacements and rejects security policy mutations', (t) => {
  const fixture = mkdtempSync(path.join(tmpdir(), 'workflow-guard-'));
  t.after(() => rmSync(fixture, {recursive: true, force: true}));
  const files = [
    'package.json', 'compose/Dockerfile', 'compose/docker-compose.yml',
    'scripts/verify-fresh-install.mjs', 'scripts/verify-backup-restore-s3.mjs',
    'scripts/check-release-workflows.mjs', 'scripts/lib/validation-files.mjs',
    ...['publish-candidate-image', 'publish-release-images', 'publish-prerelease-images',
      'quality', 'runtime', 'fresh-install', 'prepare-release'].map((name) => `.github/workflows/${name}.yml`),
  ];
  for (const file of files) {
    mkdirSync(path.dirname(path.join(fixture, file)), {recursive: true});
    copyFileSync(path.join(root, file), path.join(fixture, file));
  }
  symlinkSync(path.join(root, 'node_modules'), path.join(fixture, 'node_modules'));
  const arguments_ = ['apply', '--include=scripts/check-release-workflows.mjs', path.join(root, 'tools/workflow-release-guard.patch')];
  const applied = spawnSync('git', [...arguments_, '--reverse', '--check'], {cwd: fixture, encoding: 'utf8'});
  if (applied.status !== 0) {
    const patch = spawnSync('git', arguments_, {cwd: fixture, encoding: 'utf8'});
    assert.equal(patch.status, 0, patch.stderr);
  }
  const check = () => spawnSync(process.execPath, ['scripts/check-release-workflows.mjs'], {cwd: fixture, encoding: 'utf8'});
  assert.equal(check().status, 0, check().stderr);
  for (const [name, before, after, message] of [
    ['publish-release-images', 'f"{image}:candidate-{source}-{arch}"', 'f"{image}:latest-{arch}"', /exact AMD64/],
    ['publish-release-images', '"--tag", f"{image}:latest"', '"--tag", f"{image}:unstable"', /version and latest/],
    ['publish-release-images', '"--is-ancestor"', '"--not-an-ancestor"', /ancestry/],
    ['publish-candidate-image', '("arm64", 3)', '("arm64", 300)', /retention/],
    ['publish-candidate-image', 'if current != source:', 'if False:', /upstream dev SHA/],
    ['publish-candidate-image', 'if published != digest:', 'if False:', /published digest/],
    ['publish-candidate-image', 'if len(pieces) != 2:', 'if False:', /exactly one historical/],
    ['publish-candidate-image', 'actions/checkout@9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0', 'actions/checkout@main', /unpinned/],
    ['prepare-release', 'docker buildx imagetools inspect "$candidate"', 'echo skipped', /before pushing/],
    ['publish-prerelease-images', 'needs: [validate, promote-image, release-smoke, populated-upgrade]', 'needs: [validate, promote-image]', /wait for promoted-image verification/],
  ]) {
    const file = path.join(fixture, `.github/workflows/${name}.yml`);
    const original = readFileSync(file, 'utf8');
    assert.ok(original.includes(before), `Missing mutation seam: ${before}`);
    writeFileSync(file, original.replace(before, after));
    const result = check();
    assert.equal(result.status, 1, `${name}: ${result.stdout}`);
    assert.match(result.stderr, message);
    writeFileSync(file, original);
  }
});
