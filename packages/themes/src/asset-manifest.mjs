import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';

export async function buildAssetManifest(root, registry) {
  const occurrences = registry.builtins.flatMap((theme) => Object.entries(theme.brandAssets).flatMap(([kind, modes]) =>
    Object.entries(modes).map(([mode, publicPath]) => ({ publicPath, owner: { themeId: theme.id, kind, mode } }))));
  const grouped = Map.groupBy(occurrences, (reference) => reference.publicPath);
  const paths = Array.from(grouped.keys()).sort((left, right) => left.localeCompare(right));
  const assets = await Promise.all(paths.map(async (publicPath) => {
    const bytes = await readFile(resolve(root, 'apps/web/public', publicPath.replace(/^\//, '')));
    const source = bytes.toString('utf8');
    const externalReferences = [];
    for (const match of source.matchAll(/(?:href|src)=["']([^"']+)["']/gi)) {
      if (match[1].slice(0, 5) !== 'data:') externalReferences.push(match[1]);
    }
    return {
      publicPath,
      sha256: createHash('sha256').update(bytes).digest('hex'),
      bytes: bytes.length,
      status: /<image\b/i.test(source) ? 'raster-backed-fallback' : 'approved-vector',
      selfContained: !externalReferences.length,
      externalReferences,
      owners: grouped.get(publicPath).map((reference) => reference.owner),
    };
  }));
  return { schemaVersion: 1, assets };
}
