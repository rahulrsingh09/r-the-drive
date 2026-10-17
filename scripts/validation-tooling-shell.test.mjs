import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { setTimeout as pause } from 'node:timers/promises';
import test from 'node:test';

const repository =
  process.env.VALIDATION_BASELINE_DIR ?? fileURLToPath(new URL('..', import.meta.url));
function temp(t) {
  const root = mkdtempSync(join(tmpdir(), 'validation-shell-'));
  t.after(() => rmSync(root, { force: true, recursive: true }));
  return root;
}
function put(root, file, body, mode = 0o600) {
  mkdirSync(dirname(join(root, file)), { recursive: true });
  writeFileSync(join(root, file), body, { mode });
}
function rewritten(root, file) {
  const replacements = new Map([
    ['/app/', root + '/app/'],
    ['/backups', root + '/backups'],
    ['/data/cache', root + '/cache'],
    ['/tmp/panda-', root + '/tmp/panda-'],
  ]);
  let body = readFileSync(join(repository, file), 'utf8');
  for (const [from, to] of replacements) body = body.replaceAll(from, to);
  put(root, file, body, 0o700);
  return join(root, file);
}
async function until(predicate, message, milliseconds = 5000) {
  const deadline = Date.now() + milliseconds;
  while (!predicate()) {
    if (Date.now() > deadline) assert.fail(message);
    await pause(20);
  }
}
async function supervisor(
  t,
  { key, rendererFails = false, failProbe = false, failEntropy = false } = {}
) {
  const root = temp(t);
  for (const directory of ['tmp', 'bin', 'app', 'backups', 'cache'])
    mkdirSync(join(root, directory));
  if (key) put(root, 'backups/.restore-agent-key', key);
  put(
    root,
    'app/render-origin-gate.sh',
    `#!/bin/sh\n[ "$PANDA_GATEWAY_TOKEN" = synthetic-gateway-secret ] || exit 9\nexit ${rendererFails ? 1 : 0}\n`,
    0o700
  );
  const child = `#!/bin/bash
role=\${0##*/}
printf '%s\\n' "\${PANDA_GATEWAY_TOKEN-unset}" >> "$PROBE_ROOT/$role.tokens"
printf '%s\\n' "$BASHPID" >> "$PROBE_ROOT/$role.starts"
printf '%s\\n' '2026/10/17 [warn] 1#1: message "quoted" \\ path' >&2
trap 'exit 0' TERM INT
while [ ! -f "$PROBE_ROOT/$role.exit" ]; do sleep 0.03; done
status=$(cat "$PROBE_ROOT/$role.exit")
printf '%s\\n' done >> "$PROBE_ROOT/$role.done"
exit "$status"
`;
  put(root, 'app/panda-api', child, 0o700);
  put(root, 'app/panda-restore-agent', child, 0o700);
  put(root, 'bin/nginx', child, 0o700);
  if (failProbe) put(root, 'bin/mktemp', '#!/bin/sh\nexit 1\n', 0o700);
  if (failEntropy) put(root, 'bin/dd', '#!/bin/sh\nprintf partial-entropy\nexit 1\n', 0o700);
  const script = rewritten(root, 'compose/production-services.sh');
  let stdout = '',
    stderr = '',
    status = null;
  const process = spawn('/bin/bash', [script], {
    env: {
      PATH: root + '/bin:/usr/bin:/bin',
      PROBE_ROOT: root,
      PANDA_GATEWAY_TOKEN: 'synthetic-gateway-secret',
    },
    detached: true,
  });
  process.stdout.on('data', (chunk) => {
    stdout += chunk;
  });
  process.stderr.on('data', (chunk) => {
    stderr += chunk;
  });
  const ended = new Promise((resolve) =>
    process.on('close', (code, signal) => {
      status = { code, signal };
      resolve(status);
    })
  );
  t.after(async () => {
    if (!status) {
      try {
        globalThis.process.kill(-process.pid, 'SIGTERM');
      } catch {
        /* The process group already exited. */
      }
      await Promise.race([ended, pause(3000)]);
      if (!status) {
        try {
          globalThis.process.kill(-process.pid, 'SIGKILL');
        } catch {
          /* The process group already exited. */
        }
        await ended;
      }
    }
  });
  return {
    root,
    process,
    ended,
    result: () => status,
    output: () => stdout + stderr,
    stop: () => process.kill('SIGTERM'),
  };
}

test('health check uses primary health then only a marked restore-agent fallback', (t) => {
  const root = temp(t);
  mkdirSync(join(root, 'bin'));
  mkdirSync(join(root, 'tmp'));
  put(
    root,
    'bin/curl',
    '#!/bin/sh\nprintf "%s\\n" "$2" >> "$PROBE_ROOT/calls"\ncase "$2" in *:8080/*) exit "$PRIMARY";; *) exit "$AGENT";; esac\n',
    0o700
  );
  const script = rewritten(root, 'compose/production-healthcheck.sh');
  for (const [primary, marker, agent, expected, count] of [
    [0, false, 7, 0, 1],
    [7, false, 0, 1, 1],
    [7, true, 0, 0, 2],
    [7, true, 22, 22, 2],
  ]) {
    rmSync(join(root, 'calls'), { force: true });
    rmSync(join(root, 'tmp/panda-restore-in-progress'), { force: true });
    if (marker) put(root, 'tmp/panda-restore-in-progress', '');
    const result = spawnSync('/bin/sh', [script], {
      env: {
        PATH: root + '/bin:/usr/bin:/bin',
        PROBE_ROOT: root,
        PRIMARY: String(primary),
        AGENT: String(agent),
      },
      encoding: 'utf8',
    });
    assert.equal(result.status, expected, result.stderr);
    assert.equal(readFileSync(join(root, 'calls'), 'utf8').trim().split('\n').length, count);
  }
});

test('gateway config is atomic, case sensitive, private and preserves old config on refusal', (t) => {
  const root = temp(t),
    file = join(root, 'gate.conf');
  const token = 'Synthetic_Gateway_ABCDEFGHIJKLMNOPQRSTUVWXYZ123456';
  const run = (value, required) =>
    spawnSync('/bin/sh', [join(repository, 'compose/render-origin-gate.sh'), file], {
      env: { PATH: '/usr/bin:/bin', PANDA_GATEWAY_TOKEN: value, PANDA_REQUIRE_GATEWAY: required },
      encoding: 'utf8',
    });
  for (const required of ['true', '1', 'false', '0', '']) {
    assert.equal(run(token, required).status, 0);
    assert.equal(
      readFileSync(file, 'utf8'),
      `map $http_x_panda_edge $panda_origin_authorized {\n    default 0;\n    ~^${token}$ 1;\n}\n`
    );
  }
  const before = readFileSync(file);
  assert.equal(run('invalid token', 'true').status, 1);
  assert.deepEqual(readFileSync(file), before);
  assert.equal(run('', '0').status, 0);
  assert.equal(
    readFileSync(file, 'utf8'),
    'map $http_x_panda_edge $panda_origin_authorized {\n    default 1;\n}\n'
  );
});

test('supervisor creates private custody key, strips gateway token and normalizes child diagnostics', async (t) => {
  const run = await supervisor(t);
  await until(() => existsSync(join(run.root, 'panda-api.starts')), 'API did not start');
  const key = readFileSync(join(run.root, 'backups/.restore-agent-key'), 'utf8').trim();
  assert.equal(Buffer.from(key, 'base64').length, 32);
  assert.equal(statSync(join(run.root, 'backups/.restore-agent-key')).mode & 0o777, 0o600);
  for (const role of ['panda-api', 'panda-restore-agent', 'nginx']) {
    await until(() => existsSync(join(run.root, role + '.tokens')), role + ' did not start');
    assert.equal(readFileSync(join(run.root, role + '.tokens'), 'utf8').trim(), 'unset');
  }
  assert.match(
    run.output(),
    /\[panda\]\[WARN\] source=nginx message="message \\"quoted\\" \\\\ path"/
  );
  assert.equal(run.output().includes('synthetic-gateway-secret'), false);
  run.stop();
  assert.equal((await run.ended).code, 0);
});

test('supervisor preserves an existing custody key and propagates API exit status', async (t) => {
  const run = await supervisor(t, { key: 'existing-synthetic-key\n' });
  await until(() => existsSync(join(run.root, 'panda-api.starts')), 'API did not start');
  put(run.root, 'panda-api.exit', '7');
  await until(() => run.result(), 'supervisor did not exit');
  assert.equal(run.result().code, 7);
  assert.equal(
    readFileSync(join(run.root, 'backups/.restore-agent-key'), 'utf8'),
    'existing-synthetic-key\n'
  );
});

test('supervisor refuses storage, entropy or gateway setup failures before launching children', async (t) => {
  for (const options of [{ rendererFails: true }, { failProbe: true }, { failEntropy: true }]) {
    const run = await supervisor(t, options);
    await until(() => run.result(), 'supervisor did not refuse startup');
    assert.equal(run.result().code, 1);
    assert.equal(existsSync(join(run.root, 'panda-api.starts')), false);
  }
});

test('supervisor pauses API restart until restore marker clears', async (t) => {
  const run = await supervisor(t);
  await until(() => existsSync(join(run.root, 'panda-api.starts')), 'API did not start');
  put(run.root, 'tmp/panda-restore-in-progress', '');
  put(run.root, 'panda-api.exit', '0');
  await until(() => existsSync(join(run.root, 'panda-api.done')), 'API did not exit');
  await pause(300);
  assert.equal(
    readFileSync(join(run.root, 'panda-api.starts'), 'utf8').trim().split('\n').length,
    1
  );
  rmSync(join(run.root, 'panda-api.exit'));
  rmSync(join(run.root, 'tmp/panda-restore-in-progress'));
  await until(
    () => readFileSync(join(run.root, 'panda-api.starts'), 'utf8').trim().split('\n').length === 2,
    'API did not restart'
  );
  run.stop();
  assert.equal((await run.ended).code, 0);
});

for (const role of ['panda-restore-agent', 'nginx'])
  test(`supervisor fails when ${role} dies during restore`, async (t) => {
    const run = await supervisor(t);
    await until(() => existsSync(join(run.root, 'panda-api.starts')), 'API did not start');
    put(run.root, 'tmp/panda-restore-in-progress', '');
    put(run.root, 'panda-api.exit', '0');
    await until(() => existsSync(join(run.root, 'panda-api.done')), 'API did not exit');
    await pause(350);
    put(run.root, role + '.exit', '2');
    await until(() => run.result(), `${role} death was not detected during restore`, 2500);
    assert.equal(run.result().code, 1);
  });
