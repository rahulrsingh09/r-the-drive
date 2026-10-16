import { readFileSync, realpathSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';

const directory = fileURLToPath(new URL('.', import.meta.url));
export function renderJob() {
  const policy = Buffer.from(readFileSync(resolve(directory, '../northflank_deploy.py'))).toString('base64');
  const deploy = Buffer.from(readFileSync(resolve(directory, 'deploy.py'))).toString('base64');
  const loader = [
    'import base64,sys,types',
    "m=types.ModuleType('northflank_deploy')",
    "sys.modules['northflank_deploy']=m",
    `exec(compile(base64.b64decode('${policy}'),'installed-policy','exec'),m.__dict__)`,
    `exec(compile(base64.b64decode('${deploy}'),'installed-deployer','exec'))`,
  ].join(';');
  return {
    name: 'verified-release-controller',
    description: 'Fixed deployment controller; runs only inside the verified release workflow.',
    billing: { deploymentPlan: 'nf-compute-20' },
    deployment: {
      external: { imagePath: 'python@sha256:bf44cdfcb76cd3b41e879bc058fc37ec5872002ccfde7fcb765e218cde0cd79c' },
      docker: { configType: 'customEntrypointCustomCommand',
        customEntrypoint: 'python', customCommand: `-c "${loader}"` },
    },
    settings: { backoffLimit: 0, activeDeadlineSeconds: 720, runOnSourceChange: 'never',
      cron: { schedule: null, suspended: true, concurrencyPolicy: 'forbid' } },
  };
}

if (process.argv[1] && realpathSync(process.argv[1]) === fileURLToPath(import.meta.url)) {
  console.log(JSON.stringify(renderJob(), null, 2));
}
