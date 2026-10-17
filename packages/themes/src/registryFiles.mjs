import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { spawn } from 'node:child_process';
import { buildAssetManifest } from './asset-manifest.mjs';
import { radAssets } from './rad-assets.mjs';

export const packageDir = resolve(dirname(fileURLToPath(import.meta.url)), '..');
export const repositoryRoot = resolve(packageDir, '../..');
const jsonFile = (value) => JSON.stringify(value, null, 2) + '\n';

async function readRegistry() {
  const source = pathToFileURL(resolve(packageDir, 'src/index.ts')).href;
  const script = `import { registryManifest, stableThemeJson } from ${JSON.stringify(source)}; console.log(stableThemeJson(registryManifest()));`;
  const child = spawn(process.execPath, ['--experimental-strip-types', '--input-type=module', '-e', script], { stdio: ['ignore', 'pipe', 'pipe'] });
  const output = [], errors = [];
  child.stdout.on('data', (chunk) => output.push(chunk));
  child.stderr.on('data', (chunk) => errors.push(chunk));
  await new Promise((resolve, reject) => {
    child.once('error', reject);
    child.once('close', (code) => code === 0 ? resolve() : reject(new Error(Buffer.concat(errors).toString() || 'Unable to load theme registry')));
  });
  return JSON.parse(Buffer.concat(output).toString());
}

async function reconcile(file, expected, mode, missing, stale) {
  if (mode === 'write') {
    await mkdir(dirname(file), { recursive: true });
    await writeFile(file, expected, 'utf8');
    return;
  }
  let actual;
  try { actual = await readFile(file, 'utf8'); }
  catch (error) { if (missing) throw new Error(missing, { cause: error }); throw error; }
  if (actual.replace(/\r\n/g, '\n') !== expected) throw new Error(stale);
}

export async function processRegistryFiles(root, mode) {
  if (mode !== 'write' && mode !== 'check') throw new Error('Unknown theme file operation');
  for (const [name, source] of Object.entries(radAssets())) {
    await reconcile(resolve(root, 'apps/web/public', name), source, mode, null, `Stale RAD asset: ${name}. Run pnpm themes:generate.`);
  }
  const manifest = await readRegistry();
  const target = resolve(root, 'apps/api/src/themes/builtins.generated.json');
  await reconcile(target, jsonFile(manifest), mode, `Missing generated theme manifest: ${target}`, `Generated theme manifest is stale: ${target}. Run pnpm themes:generate.`);
  const assetTarget = resolve(root, 'packages/themes/generated/assets.generated.json');
  const assets = await buildAssetManifest(root, manifest);
  await reconcile(assetTarget, jsonFile(assets), mode, `Missing generated asset manifest: ${assetTarget}`, `Generated theme asset manifest is stale: ${assetTarget}. Run pnpm themes:generate.`);
  return [target, assetTarget];
}
