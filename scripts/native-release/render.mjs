import { readFileSync, realpathSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { POLICY } from '../fork-deploy-readiness.mjs';

const encode = text => Buffer.from(text).toString('base64');
const shell = script => "sh -c '" + script.split("'").join("'\\''") + "'";
function action(ref, command, dispatchOnly, skipNodeExecution) {
  const node = { kind: 'Action', ref, spec: { kind: 'Service', spec: {
    type: 'execute', data: { serviceId: 'telemetry-app', command, options: { dispatchOnly } },
  } } };
  if (skipNodeExecution !== undefined) node.skipNodeExecution = skipNodeExecution;
  return node;
}

export function guardWaitSteps(phase, guardScript) {
  if (!new Set(['preflight', 'backup', 'verify']).has(phase)) throw new Error('Invalid guard phase');
  const source = ["GUARD_B64='" + encode(guardScript) + "'", readFileSync(new URL('wait.sh', import.meta.url), 'utf8')].join('\n');
  const encoded = encode(source);
  const digest = createHash('sha256').update(source).digest('hex');
  const args = `'${phase}' '\${fn.toBase64(args.sha)}' '\${fn.toBase64(args.runId)}'`;
  function resume(mode) {
    const lines = [
      'set -eu',
      `op=$(printf %s '\${fn.toBase64(refs.${phase}Prepare.stdOut)}' | base64 -d | sed -n 's/^PANDA_ASYNC_OP \\(op\\.[a-zA-Z0-9]*\\)$/\\1/p')`,
      'case "$op" in op.*) ;; *) exit 1 ;; esac',
      'suffix=$(printf %s "$op" | cut -c4-)', 'test "$(printf %s "$suffix" | wc -c)" -eq 16',
      'case "$suffix" in *[!a-zA-Z0-9]*) exit 1 ;; esac',
      'path="/backups/native-release/async/$op/control.sh"', 'test -f "$path"', 'test ! -L "$path"',
      `test "$(sha256sum "$path" | cut -d' ' -f1)" = '${digest}'`,
      `exec sh "$path" '${mode}' ${args} "$(printf %s "$op" | base64 -w0)"`,
    ];
    return shell(lines.join('; '));
  }
  const prepare = shell(`CONTROL_B64='${encoded}'; export CONTROL_B64; printf %s '${encoded}' | base64 -d | sh -s -- prepare ${args}`);
  const steps = [action(phase + 'Prepare', prepare, false), action(phase + 'Dispatch', resume('worker'), true)];
  const pollCommand = resume('poll');
  const polls = Array.from({ length: 60 }, (_, index) => {
    const previous = `refs.${phase}Poll${index - 1}.stdOut`;
    const skip = index > 0 ? `\${fn.eq(fn.indexOf(fn.if(${previous}, ${previous}, ''), 'PANDA_ASYNC_PENDING'), -1)}` : undefined;
    return action(phase + 'Poll' + index, pollCommand, false, skip);
  });
  return steps.concat(polls, action(phase, resume('collect'), false));
}

export function renderWorkflow() {
  const read = relative => readFileSync(new URL(relative, import.meta.url), 'utf8');
  const approval = JSON.parse(read('../../config/native-release-catalog.json'));
  const guard = `EXPECTED_LEDGER='${JSON.stringify(approval)}'\n` + read('guard.sh');
  const build = { kind: 'Build', ref: 'build', condition: 'success', spec: {
    type: 'service', id: 'telemetry-app', branch: POLICY.branch, sha: '${args.sha}',
    reuseExistingBuilds: true, buildRuleFallThroughHandling: 'fail',
    buildOverrides: { buildArguments: { PANDA_RELEASE_SHA: '${args.sha}' } },
  } };
  const deploy = { kind: 'JobRun', ref: 'deploy', condition: 'success', spec: {
    jobId: 'verified-release-controller', runtimeEnvironment: {
      RELEASE_SHA: '${args.sha}', CI_RUN_ID: '${args.runId}', BUILD_ID: '${refs.build.id}', BACKUP_ATTESTATION: '${refs.backup.stdOut}',
    },
  } };
  const running = { kind: 'Condition', ref: 'running', spec: { kind: 'Service', spec: {
    type: 'running', data: { serviceId: 'telemetry-app', timeoutDuration: 600 },
  } } };
  const steps = [];
  for (const stage of ['preflight', build, 'backup', deploy, running, 'verify']) {
    if (typeof stage === 'string') steps.push(...guardWaitSteps(stage, guard));
    else steps.push(stage);
  }
  return { name: 'panda-verified-release',
    description: 'Protected GitHub CI, backup, pinned deployment, private access and data checks.',
    apiVersion: 'v1.2', arguments: { sha: '', runId: '' }, options: { autorun: false, concurrencyPolicy: 'queue' },
    spec: { kind: 'Workflow', spec: { type: 'sequential', steps } } };
}
if (process.argv[1] && fileURLToPath(import.meta.url) === realpathSync(process.argv[1]))
  console.log(JSON.stringify(renderWorkflow(), null, 2));
