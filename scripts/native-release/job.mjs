import { readFileSync, realpathSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

export function renderJob() {
  const modules = [
    ['../northflank_deploy.py', 'installed-policy', ',m.__dict__'],
    ['deploy.py', 'installed-deployer', ''],
  ];
  const commands = ['import base64,sys,types', "m=types.ModuleType('northflank_deploy')", "sys.modules['northflank_deploy']=m"];
  for (const [path, label, scope] of modules) {
    const data = readFileSync(new URL(path, import.meta.url)).toString('base64');
    commands.push(`exec(compile(base64.b64decode('${data}'),'${label}','exec')${scope})`);
  }
  const job = {
    name: 'verified-release-controller',
    description: 'Fixed deployment controller; runs only inside the verified release workflow.',
    billing: { deploymentPlan: 'nf-compute-20' },
    deployment: {
      external: { imagePath: 'python@sha256:bf44cdfcb76cd3b41e879bc058fc37ec5872002ccfde7fcb765e218cde0cd79c' },
      docker: { configType: 'customEntrypointCustomCommand', customEntrypoint: 'python',
        customCommand: `-c "${commands.join(';')}"` },
    },
    settings: { backoffLimit: 0, activeDeadlineSeconds: 720, runOnSourceChange: 'never',
      cron: { schedule: null, suspended: true, concurrencyPolicy: 'forbid' } },
  };
  return job;
}
if (process.argv[1] && fileURLToPath(import.meta.url) === realpathSync(process.argv[1]))
  console.log(JSON.stringify(renderJob(), null, 2));
