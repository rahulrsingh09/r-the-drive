import assert from 'node:assert/strict';
import {copyFileSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {dirname, join} from 'node:path';
import {fileURLToPath} from 'node:url';
import test from 'node:test';
import {releaseWorkflowFailures} from './check-release-workflows.mjs';
import {FreshInstall} from './verify-fresh-install.mjs';
import {S3RestoreDrill} from './verify-backup-restore-s3.mjs';

const repository = fileURLToPath(new URL('..', import.meta.url));

function temporary(t) {
  const directory = mkdtempSync(join(tmpdir(), 'operational-release-contract-'));
  t.after(() => rmSync(directory, {recursive: true, force: true}));
  return directory;
}

function guardFixture(t) {
  const directory = temporary(t);
  const files = [
    'package.json', 'compose/Dockerfile', 'compose/docker-compose.yml',
    'scripts/verify-fresh-install.mjs', 'scripts/verify-backup-restore-s3.mjs',
    ...readdirSync(join(repository, '.github/workflows')).map(name => '.github/workflows/' + name),
  ];
  for (const file of files) {
    mkdirSync(dirname(join(directory, file)), {recursive: true});
    copyFileSync(join(repository, file), join(directory, file));
  }
  return directory;
}

test('release guards accept characterized drivers and reject loss of each protected operation', async t => {
  const directory = guardFixture(t);
  assert.deepEqual(releaseWorkflowFailures(directory), []);
  const mutations = [
    ['verify-fresh-install.mjs', "'image-ref',", "'unused',", /must accept --image-ref/],
    ['verify-fresh-install.mjs', 'this.environment.PANDA_IMAGE = image', 'this.environment.PANDA_IMAGE = "other"', /exact image reference/],
    ['verify-fresh-install.mjs', "this.command(['restart', 'panda']);", '', /pre-restart access tokens/],
    ['verify-backup-restore-s3.mjs', 'chown 1001:1001 /data/backups /data/cache', 'true', /prepare the source bind mount/],
    ['verify-backup-restore-s3.mjs', "this.stack('target', 2000)", "this.unpreparedTarget('target', 2000)", /prepare the target bind mount/],
    ['verify-backup-restore-s3.mjs', 'await this.backup(source, sourceToken, backup.run.id);', '', /wait for the asynchronous backup/],
    ['verify-backup-restore-s3.mjs', "!stores.has('local') || !stores.has('s3')", "!stores.has('local')", /both local and S3 artifacts/],
    ['verify-backup-restore-s3.mjs', 'panda_restore_agent=error', 'panda_restore_agent=off', /restore-agent error diagnostics/],
  ];
  for (const [name, before, after, message] of mutations) {
    await t.test(message.source, () => {
      const file = join(directory, 'scripts', name);
      const original = readFileSync(file, 'utf8');
      assert(original.includes(before), before);
      try {
        writeFileSync(file, original.replace(before, after));
        assert.match(releaseWorkflowFailures(directory).join('\n'), message);
      } finally {
        writeFileSync(file, original);
      }
    });
  }
});

test('fresh production verification sends the selected digest to Compose and reuses the owner token after restart', async t => {
  const directory = temporary(t);
  const environmentFile = join(directory, 'production.env');
  writeFileSync(environmentFile, 'PANDA_ENV=production\n');
  const image = 'registry.example.test/drive@sha256:' + 'a'.repeat(64);
  const events = [];
  const commands = {
    invoke(program, args, options) {
      events.push({program, args, image: options.env.PANDA_IMAGE});
      return '';
    },
  };
  const fresh = new FreshInstall(['--production-env', environmentFile, '--image-ref', image], {
    commands, env: {}, fetchImpl: async () => ({json: async () => ({setup_required: false})}),
  });
  fresh.owner = async origin => {
    events.push({owner: origin});
    return 'synthetic-before-restart-token';
  };
  fresh.healthy = async origin => { events.push({healthy: origin}); };
  fresh.dashboards = async (origin, token) => { events.push({dashboards: origin, token}); };
  t.after(() => {
    if (fresh.dataRoot) rmSync(fresh.dataRoot, {recursive: true, force: true});
  });
  await fresh.production();
  const commandsSeen = events.filter(event => event.program);
  assert.equal(commandsSeen.length, 3);
  assert(commandsSeen.every(event => event.program === 'docker' && event.image === image));
  const restart = events.findIndex(event => event.args?.includes('restart'));
  const dashboard = events.findIndex(event => event.dashboards);
  assert(restart >= 0 && dashboard > restart);
  assert.equal(events[dashboard].token, 'synthetic-before-restart-token');
});

test('both S3 drill stacks prepare owned bind mounts before startup and preserve restore diagnostics', t => {
  const calls = [];
  const drill = new S3RestoreDrill([], {
    commands: {invoke(program, args, options = {}) { calls.push({program, args, options}); return ''; }},
  });
  t.after(() => rmSync(drill.directory, {recursive: true, force: true}));
  for (const [name, offset] of [['source', 1000], ['target', 2000]]) {
    const start = calls.length;
    const stack = drill.stack(name, offset);
    assert.equal(calls.length, start + 2);
    const [prepare, launch] = calls.slice(start);
    assert.equal(prepare.program, 'docker');
    assert(prepare.args.includes(`type=bind,source=${stack.directory},target=/data`));
    assert.match(prepare.args.at(-1), /chown 1001:1001 \/data\/backups \/data\/cache/);
    assert.deepEqual(launch.args.slice(-2), ['up', '-d']);
    assert.equal(launch.options.env.PANDA_DATA_DIR, stack.directory);
    assert.match(readFileSync(stack.envFile, 'utf8'),
      /^RUST_LOG=panda_api=info,panda_restore_agent=error,tower_http=info$/m);
  }
});
