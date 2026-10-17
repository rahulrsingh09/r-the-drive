import { afterEach, expect, it } from 'vitest';
import { cp, mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { buildAssetManifest } from '../../../../packages/themes/src/asset-manifest.mjs';
import { processRegistryFiles } from '../../../../packages/themes/src/registryFiles.mjs';

const root = resolve(process.cwd(), '../..');
const temporary = [];
afterEach(async () => { for (const path of temporary.splice(0)) await rm(path, { recursive: true, force: true }); });

it('generates and checks isolated files without changing shipped assets', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'fullReplacementHooksTypesThemes-')); temporary.push(fixture);
  const publicDir = join(fixture, 'apps/web/public');
  await mkdir(publicDir, { recursive: true });
  for (const name of ['text_white.svg', 'text_black.svg', 'logo_color_lighter.svg', 'icon_color_lighter.svg', 'favicon.svg']) await cp(join(root, 'apps/web/public', name), join(publicDir, name));
  const targets = await processRegistryFiles(fixture, 'write');
  const generated = JSON.parse(await readFile(targets[0], 'utf8'));
  const shipped = JSON.parse(await readFile(join(root, 'apps/api/src/themes/builtins.generated.json'), 'utf8'));
  expect(generated).toEqual(shipped);
  await expect(processRegistryFiles(fixture, 'check')).resolves.toEqual(targets);
  await writeFile(join(publicDir, 'rad-icon.svg'), 'stale');
  await expect(processRegistryFiles(fixture, 'check')).rejects.toThrow('Stale RAD asset: rad-icon.svg');
});

it('groups owners and diagnoses embedded images and non-data references deterministically', async () => {
  const fixture = await mkdtemp(join(tmpdir(), 'fullReplacementHooksTypesThemes-')); temporary.push(fixture);
  const publicDir = join(fixture, 'apps/web/public'); await mkdir(publicDir, { recursive: true });
  await writeFile(join(publicDir, 'vector.svg'), '<svg><path d="M0 0"/></svg>');
  await writeFile(join(publicDir, 'raster.svg'), '<svg><image href="data:image/png;base64,AA"/><use href="/local.svg"/><image src="https://invalid.test/image"/></svg>');
  const manifest = await buildAssetManifest(fixture, { builtins: [{ id: 'synthetic', brandAssets: { icon: { dark: '/vector.svg', light: '/vector.svg' }, logo: { light: '/raster.svg' } } }] });
  expect(manifest.assets.map(({ publicPath, status, selfContained, externalReferences, owners }) => ({ publicPath, status, selfContained, externalReferences, owners }))).toEqual([
    { publicPath: '/raster.svg', status: 'raster-backed-fallback', selfContained: false, externalReferences: ['/local.svg', 'https://invalid.test/image'], owners: [{ themeId: 'synthetic', kind: 'logo', mode: 'light' }] },
    { publicPath: '/vector.svg', status: 'approved-vector', selfContained: true, externalReferences: [], owners: [{ themeId: 'synthetic', kind: 'icon', mode: 'dark' }, { themeId: 'synthetic', kind: 'icon', mode: 'light' }] },
  ]);
});
