import assert from 'node:assert/strict';
import {
  mkdtempSync,
  mkdirSync,
  writeFileSync,
  rmSync,
  readFileSync,
  copyFileSync,
  symlinkSync,
} from 'node:fs';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import test from 'node:test';

const repo = fileURLToPath(new URL('..', import.meta.url));
const source = process.env.VALIDATION_BASELINE_DIR ?? repo;
const { architectureGuardFailures } = await import(
  pathToFileURL(join(source, 'scripts/check-architecture-guards.mjs'))
);
function fixture(t) {
  const root = mkdtempSync(join(tmpdir(), 'validation-regression-'));
  t.after(() => rmSync(root, { force: true, recursive: true }));
  return root;
}
function put(root, path, value) {
  mkdirSync(dirname(join(root, path)), { recursive: true });
  writeFileSync(join(root, path), value);
}

test('a global architecture pattern reports every matching file, without regex state leaking between files', (t) => {
  const root = fixture(t);
  put(root, 'apps/web/src/a.ts', 'window.alert("x");');
  put(root, 'apps/web/src/b.ts', 'window.alert("y");');
  put(
    root,
    'config/architecture-budgets.json',
    JSON.stringify({
      patterns: {
        alert: {
          pattern: 'window\\.alert',
          flags: 'g',
          directories: ['apps/web/src'],
          allow: ['apps/web/src/a.ts'],
        },
      },
    })
  );
  assert.match(architectureGuardFailures(root).join('\n'), /apps\/web\/src\/b.ts/);
});

test('release action pin validation rejects a mutable tag even when no other contract mentions that action', (t) => {
  const root = fixture(t);
  for (const directory of ['.github/workflows', 'compose', 'scripts', 'scripts/lib'])
    mkdirSync(join(root, directory), { recursive: true });
  const files = [
    'package.json',
    'compose/Dockerfile',
    'compose/docker-compose.yml',
    'scripts/verify-fresh-install.mjs',
    'scripts/verify-backup-restore-s3.mjs',
  ];
  files.push(
    ...[
      'publish-candidate-image.yml',
      'publish-release-images.yml',
      'publish-prerelease-images.yml',
      'quality.yml',
      'runtime.yml',
      'fresh-install.yml',
      'prepare-release.yml',
    ].map((name) => '.github/workflows/' + name)
  );
  for (const file of files) copyFileSync(join(repo, file), join(root, file));
  copyFileSync(
    join(source, 'scripts/check-release-workflows.mjs'),
    join(root, 'scripts/check-release-workflows.mjs')
  );
  copyFileSync(
    join(repo, 'scripts/lib/validation-files.mjs'),
    join(root, 'scripts/lib/validation-files.mjs')
  );
  symlinkSync(join(repo, 'node_modules'), join(root, 'node_modules'), 'dir');
  const path = join(root, '.github/workflows/publish-candidate-image.yml');
  const body = readFileSync(path, 'utf8');
  assert.match(body, /actions\/checkout@[a-f0-9]{40}/);
  writeFileSync(path, body.replace(/actions\/checkout@[a-f0-9]{40}/, 'actions/checkout@v6'));
  const result = spawnSync(process.execPath, ['scripts/check-release-workflows.mjs'], {
    cwd: root,
    encoding: 'utf8',
  });
  assert.equal(result.status, 1, result.stdout + result.stderr);
  assert.match(result.stderr, /unpinned or short action SHA: v6/);
});
