import { readFileSync, realpathSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';
import { createHash } from 'node:crypto';
import { POLICY } from '../fork-deploy-readiness.mjs';

const directory = fileURLToPath(new URL('.', import.meta.url));

export function guardWaitSteps(phase, guardScript) {
  if (!['preflight', 'backup', 'verify'].includes(phase)) throw new Error('Invalid guard phase');
  const guard = Buffer.from(guardScript).toString('base64');
  const control = `GUARD_B64='${guard}'\n${readFileSync(resolve(directory, 'wait.sh'), 'utf8')}`;
  const encoded = Buffer.from(control).toString('base64');
  const digest = createHash('sha256').update(control).digest('hex');
  const action = (ref, command, dispatchOnly = false, skipNodeExecution) => ({
    kind: 'Action', ref,
    ...(skipNodeExecution === undefined ? {} : { skipNodeExecution }),
    spec: { kind: 'Service', spec: { type: 'execute', data: {
      serviceId: 'telemetry-app',
      command, options: { dispatchOnly },
    } } },
  });
  const args = `'${phase}' '\${fn.toBase64(args.sha)}' '\${fn.toBase64(args.runId)}'`;
  const command = (mode) => {
    const encodedOutput = `'\${fn.toBase64(refs.${phase}Prepare.stdOut)}'`;
    const launcher = `set -eu; op=$(printf %s ${encodedOutput} | base64 -d | ` +
      `sed -n 's/^PANDA_ASYNC_OP \\(op\\.[a-zA-Z0-9]*\\)$/\\1/p'); ` +
      `case "$op" in op.*) ;; *) exit 1 ;; esac; suffix=$(printf %s "$op" | cut -c4-); ` +
      `test "$(printf %s "$suffix" | wc -c)" -eq 16; ` +
      `case "$suffix" in *[!a-zA-Z0-9]*) exit 1 ;; esac; ` +
      `path="/backups/native-release/async/$op/control.sh"; test -f "$path"; test ! -L "$path"; ` +
      `test "$(sha256sum "$path" | cut -d' ' -f1)" = '${digest}'; ` +
      `exec sh "$path" '${mode}' ${args} "$(printf %s "$op" | base64 -w0)"`;
    return `sh -c '${launcher.replaceAll("'", "'\\''")}'`;
  };
  const prepare = `CONTROL_B64='${encoded}'; export CONTROL_B64; ` +
    `printf %s '${encoded}' | base64 -d | sh -s -- prepare ${args}`;
  const steps = [
    action(`${phase}Prepare`, `sh -c '${prepare.replaceAll("'", "'\\''")}'`),
    action(`${phase}Dispatch`, command('worker'), true),
  ];
  for (let index = 0; index < 60; index++) {
    const previous = `refs.${phase}Poll${index - 1}.stdOut`;
    steps.push(action(`${phase}Poll${index}`, command('poll'), false,
      index === 0 ? undefined :
        `\${fn.eq(fn.indexOf(fn.if(${previous}, ${previous}, ''), 'PANDA_ASYNC_PENDING'), -1)}`));
  }
  steps.push(action(phase, command('collect')));
  return steps;
}

export function renderWorkflow() {
  const ledger = JSON.parse(readFileSync(resolve(directory, '../../config/native-release-catalog.json'), 'utf8'));
  const script = `EXPECTED_LEDGER='${JSON.stringify(ledger)}'\n${readFileSync(resolve(directory, 'guard.sh'), 'utf8')}`;
  const phaseSteps = phase => guardWaitSteps(phase, script);
  return {
    name: 'panda-verified-release',
    description: 'Protected GitHub CI, backup, pinned deployment, private access and data checks.',
    apiVersion: 'v1.2',
    arguments: { sha: '', runId: '' },
    options: { autorun: false, concurrencyPolicy: 'queue' },
    spec: { kind: 'Workflow', spec: { type: 'sequential', steps: [
      ...phaseSteps('preflight'),
      { kind: 'Build', ref: 'build', condition: 'success', spec: {
        type: 'service', id: 'telemetry-app', branch: POLICY.branch, sha: '${args.sha}',
        reuseExistingBuilds: true, buildRuleFallThroughHandling: 'fail',
        buildOverrides: { buildArguments: { PANDA_RELEASE_SHA: '${args.sha}' } },
      } },
      ...phaseSteps('backup'),
      { kind: 'JobRun', ref: 'deploy', condition: 'success', spec: {
        jobId: 'verified-release-controller',
        runtimeEnvironment: {
          RELEASE_SHA: '${args.sha}', CI_RUN_ID: '${args.runId}',
          BUILD_ID: '${refs.build.id}', BACKUP_ATTESTATION: '${refs.backup.stdOut}',
        },
      } },
      { kind: 'Condition', ref: 'running', spec: { kind: 'Service', spec: {
        type: 'running', data: { serviceId: 'telemetry-app', timeoutDuration: 600 },
      } } },
      ...phaseSteps('verify'),
    ] } },
  };
}

if (process.argv[1] && realpathSync(process.argv[1]) === fileURLToPath(import.meta.url)) {
  console.log(JSON.stringify(renderWorkflow(), null, 2));
}
