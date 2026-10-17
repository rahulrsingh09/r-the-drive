import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { copyFileSync, mkdirSync, mkdtempSync, renameSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

const repo = fileURLToPath(new URL('..', import.meta.url));
const sourceRoot = process.env.VALIDATION_BASELINE_DIR ?? repo;
const env = {
  PATH: process.env.PATH,
  HOME: tmpdir(),
  GIT_CONFIG_GLOBAL: '/dev/null',
  GIT_CONFIG_NOSYSTEM: '1',
  GIT_AUTHOR_NAME: 'Project maintainers',
  GIT_COMMITTER_NAME: 'Project maintainers',
  GIT_AUTHOR_EMAIL: 'maintainers@example.invalid',
  GIT_COMMITTER_EMAIL: 'maintainers@example.invalid',
};
function fixture(t, count = 1) {
  const root = mkdtempSync(join(tmpdir(), 'validation-migration-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  for (const file of [
    'tools/migration-integrity.mjs',
    'scripts/resolve-build-version.mjs',
    'scripts/next-release-version.mjs',
    'scripts/lib/validation-files.mjs',
    'scripts/lib/calendar-versions.mjs',
  ]) {
    mkdirSync(dirname(join(root, file)), { recursive: true });
    copyFileSync(join(file.includes('/lib/') ? repo : sourceRoot, file), join(root, file));
  }
  mkdirSync(join(root, 'apps/api/migrations'), { recursive: true });
  for (let n = 1; n <= count; n += 1)
    writeFileSync(
      join(root, `apps/api/migrations/${String(n).padStart(4, '0')}_initial.sql`),
      `SELECT ${n};\n`
    );
  const git = (args) =>
    execFileSync('git', args, {
      cwd: root,
      env,
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'pipe'],
    }).trim();
  git(['init', '--initial-branch=main']);
  git(['add', '.']);
  git(['-c', 'core.hooksPath=/dev/null', 'commit', '-qm', 'Fixture baseline']);
  return { root, git, base: git(['rev-parse', 'HEAD']) };
}
function check(root, file = 'tools/migration-integrity.mjs', args = []) {
  const result = spawnSync(process.execPath, [join(root, file), ...args], {
    cwd: root,
    env,
    encoding: 'utf8',
  });
  return { ...result, output: result.stdout + result.stderr };
}

test('migration command accepts byte-identical history and append, refuses edit deletion and version reuse', (t) => {
  const { root, base } = fixture(t, 2);
  const script = 'tools/migration-integrity.mjs';
  const run = () => check(root, script, ['--base-ref', base]);
  assert.equal(run().status, 0);
  const first = join(root, 'apps/api/migrations/0001_initial.sql');
  writeFileSync(first, 'SELECT 100;\n');
  assert.match(run().stderr, /modified previously merged migration/);
  writeFileSync(first, 'SELECT 1;\n');
  const second = join(root, 'apps/api/migrations/0002_initial.sql');
  rmSync(second);
  assert.match(run().stderr, /deleted previously merged migration/);
  writeFileSync(second, 'SELECT 2;\n');
  renameSync(second, join(root, 'apps/api/migrations/0002_renamed.sql'));
  const reuse = run();
  assert.equal(reuse.status, 1);
  assert.match(reuse.stderr, /without appending after version 2/);
  renameSync(join(root, 'apps/api/migrations/0002_renamed.sql'), second);
  writeFileSync(join(root, 'apps/api/migrations/0003_append.sql'), 'SELECT 3;\n');
  const append = run();
  assert.equal(append.status, 0, append.output);
  assert.match(append.stdout, /3 migration\(s\), catalog digest [a-f0-9]{64}/);
});

test('migration command reports encoding, BOM, NUL, empty, newline and CR refusal classes', (t) => {
  const { root } = fixture(t);
  const file = join(root, 'apps/api/migrations/0001_initial.sql');
  for (const [bytes, pattern] of [
    [Buffer.from([255, 10]), /not valid UTF-8/],
    [Buffer.from([239, 187, 191, 10]), /BOM/],
    [Buffer.from([0, 10]), /NUL/],
    [Buffer.alloc(0), /empty/],
    [Buffer.from('SELECT 1;'), /end with one LF/],
    [Buffer.from('SELECT 1;\r\n'), /CRLF or a lone CR/],
  ]) {
    writeFileSync(file, bytes);
    const result = check(root);
    assert.equal(result.status, 1);
    assert.match(result.stderr, pattern);
  }
});

test('migration command requires a version-one, unique, contiguous catalog and valid filenames', (t) => {
  const { root } = fixture(t);
  const directory = join(root, 'apps/api/migrations');
  renameSync(join(directory, '0001_initial.sql'), join(directory, '0002_initial.sql'));
  assert.match(check(root).stderr, /start at version 0001/);
  writeFileSync(join(directory, '0001_initial.sql'), 'SELECT 1;\n');
  writeFileSync(join(directory, '0001_duplicate.sql'), 'SELECT 1;\n');
  assert.match(check(root).stderr, /duplicate migration version/);
  rmSync(join(directory, '0001_duplicate.sql'));
  renameSync(join(directory, '0002_initial.sql'), join(directory, '0003_initial.sql'));
  assert.match(check(root).stderr, /contiguous and ordered/);
  rmSync(join(directory, '0003_initial.sql'));
  renameSync(join(directory, '0001_initial.sql'), join(directory, '0001_Bad.sql'));
  assert.match(check(root).stderr, /lowercase names/);
  rmSync(join(directory, '0001_Bad.sql'));
  assert.match(check(root).stderr, /no SQL migrations/);
});

test('historical five-to-one cutover remains narrow and requires replacement baseline bytes', (t) => {
  const { root, base } = fixture(t, 5);
  for (let n = 2; n <= 5; n += 1) rmSync(join(root, `apps/api/migrations/000${n}_initial.sql`));
  assert.match(
    check(root, 'tools/migration-integrity.mjs', ['--base-ref', base]).stderr,
    /cutover must replace/
  );
  writeFileSync(join(root, 'apps/api/migrations/0001_initial.sql'), 'SELECT 100;\n');
  const cutover = check(root, 'tools/migration-integrity.mjs', ['--base-ref', base]);
  assert.equal(cutover.status, 0, cutover.output);
  assert.match(cutover.stderr, /explicit one-time baseline cutover/);
  writeFileSync(join(root, 'apps/api/migrations/0002_initial.sql'), 'SELECT 2;\n');
  assert.match(
    check(root, 'tools/migration-integrity.mjs', ['--base-ref', base]).stderr,
    /modified previously merged migration/
  );
});

test('version commands use local Git tags, candidate distance, stable spelling and usage failures', (t) => {
  const { root, git } = fixture(t);
  const script = 'scripts/resolve-build-version.mjs';
  assert.equal(check(root, script).stdout.trim(), 'unknown');
  git(['tag', '2026.10.2']);
  assert.equal(check(root, script).stdout.trim(), '2026.10.2');
  writeFileSync(join(root, 'fixture.txt'), 'change');
  git(['add', '.']);
  git(['-c', 'core.hooksPath=/dev/null', 'commit', '-qm', 'Fixture change']);
  assert.equal(check(root, script, ['--candidate']).stdout.trim(), '2026.10.2+dev.1');
  assert.equal(
    check(root, 'scripts/next-release-version.mjs', [
      '--month',
      '2026.10',
      '2026.10.2',
      'not-release',
    ]).stdout.trim(),
    '2026.10.3'
  );
  assert.notEqual(check(root, 'scripts/next-release-version.mjs').status, 0);
});
