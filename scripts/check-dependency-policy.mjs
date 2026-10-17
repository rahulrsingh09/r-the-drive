#!/usr/bin/env node
import { readFileSync } from 'node:fs';
import { basename, dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parse } from 'yaml';
import { isMain, walkFiles } from './lib/validation-files.mjs';

const ignored = new Set([
  '.agents',
  '.claude',
  '.codex',
  '.git',
  'node_modules',
  'dist',
  'coverage',
  '.turbo',
  'storybook-static',
]);

export function dependencyPolicyFailures(
  root,
  { catalogsOnly = false, today = new Date().toISOString().slice(0, 10) } = {}
) {
  const read = (path) => readFileSync(join(root, path), 'utf8');
  const json = (path) => JSON.parse(read(path));
  const workspaceText = read('pnpm-workspace.yaml');
  const workspace = parse(workspaceText);
  const failures = [];
  const catalogMatch = workspaceText.match(/^catalog:\r?\n((?:^[ ]{2}.+\r?\n?)*)/m);
  if (!catalogMatch) failures.push('pnpm-workspace.yaml must define a default catalog.');
  const catalog = new Set(Object.keys(workspace.catalog ?? {}));
  for (const file of walkFiles(root, {
    ignored,
    accept: (file) => basename(file) === 'package.json',
  })) {
    const manifest = JSON.parse(readFileSync(file, 'utf8'));
    for (const section of ['dependencies', 'devDependencies', 'optionalDependencies']) {
      const violations = Object.entries(manifest[section] ?? {}).filter(
        ([name, value]) => catalog.has(name) && value !== 'catalog:'
      );
      failures.push(
        ...violations.map(
          ([name]) => `${relative(root, file)}: ${name} must use catalog: in ${section}.`
        )
      );
    }
  }
  if (catalogsOnly) return failures;
  const fixedPolicy = [
    [
      Object.keys(workspace.auditConfig ?? {}).length === 0,
      'Native pnpm audit ignores hide findings; use config/dependency-audit-exceptions.json.',
    ],
    [
      JSON.stringify(workspace.allowBuilds) === JSON.stringify({ 'core-js': false, esbuild: true }),
      'Review dependency install scripts before changing the core-js/esbuild build allowlist.',
    ],
    [
      !workspace.patchedDependencies?.['image-size@2.0.2'] &&
        workspace.overrides?.['image-size'] === '2.0.4',
      'Use the published image-size 2.0.4 fix, not the retired partial patch.',
    ],
    [
      !/name = "proc-macro-error(?:2|-attr2)?"/.test(read('apps/api/Cargo.lock')),
      'Unmaintained proc-macro-error returned; review the age/i18n dependency path.',
    ],
  ];
  failures.push(...fixedPolicy.filter(([valid]) => !valid).map(([, message]) => message));
  for (const exception of json('config/dependency-audit-exceptions.json')) {
    if (!exception.expires || exception.expires < today)
      failures.push(
        `${exception.id}: audit exception expired ${exception.expires}; reassess applicability and upstream fixes.`
      );
  }
  const baseline = json('config/dependency-baselines.json');
  const manifest = json('package.json');
  const pnpm = manifest.packageManager?.match(/^pnpm@([^+]+)/)?.[1];
  if (pnpm !== baseline.pnpm)
    failures.push(`packageManager pnpm ${pnpm ?? 'missing'} does not match ${baseline.pnpm}.`);
  const nodePrefix = '>=' + baseline.node.split('.').slice(0, 2).join('.');
  if (!manifest.engines?.node?.startsWith(nodePrefix))
    failures.push(`Node engine must start with ${nodePrefix}.`);
  const requirements = new Map([
    ['rust-toolchain.toml', [`channel = "${baseline.rust}"`]],
    ['apps/api/Cargo.toml', [`rust-version = "${baseline.rust.split('.').slice(0, 2).join('.')}"`]],
    [
      'compose/Dockerfile',
      [
        `rust:${baseline.rust}-slim`,
        `node:${baseline.node}-alpine`,
        `postgres:${baseline.postgres}-bookworm`,
      ],
    ],
    [
      'apps/api/Dockerfile',
      [`rust:${baseline.rust}-slim`, `postgres:${baseline.postgres}-bookworm`],
    ],
    [
      'compose/docker-compose.yml',
      [`timescale/timescaledb:${baseline.timescaledb}`, `redis:${baseline.redis}`],
    ],
    [
      'compose/docker-compose.dev.yml',
      [
        `timescale/timescaledb:${baseline.timescaledb}`,
        `redis:${baseline.redis}`,
        `dxflrs/garage:v${baseline.garage}`,
      ],
    ],
  ]);
  for (const [file, snippets] of requirements) {
    const source = read(file);
    failures.push(
      ...snippets
        .filter((text) => !source.includes(text))
        .map((text) => `${file} must contain ${text}.`)
    );
  }
  for (const file of walkFiles(join(root, '.github/workflows'), {
    ignored,
    accept: (file) => /\.ya?ml$/.test(file),
  })) {
    const source = readFileSync(file, 'utf8');
    const name = relative(root, file);
    for (const match of source.matchAll(
      /(NODE|PNPM|RUST)_VERSION:\s*['"]?([0-9]+(?:\.[0-9]+(?:\.[0-9]+)?)?)/g
    )) {
      const expected = baseline[match[1].toLowerCase()];
      if (match[2] !== expected)
        failures.push(`${name} pins ${match[1]}_VERSION ${match[2]}; expected ${expected}.`);
    }
    for (const match of source.matchAll(/^\s*-\s+uses:\s+[^@\s]+@([^\s#]+)/gm)) {
      if (!/^[a-f0-9]{40}$/i.test(match[1]))
        failures.push(`${name} uses a mutable action reference: ${match[1]}.`);
    }
  }
  return failures;
}

if (isMain(import.meta.url)) {
  const catalogsOnly = process.argv.slice(2).includes('--catalogs-only');
  const failures = dependencyPolicyFailures(
    resolve(dirname(fileURLToPath(import.meta.url)), '..'),
    { catalogsOnly }
  );
  if (!failures.length)
    console.log(
      catalogsOnly ? 'Dependency catalogs are consistent.' : 'Dependency policy is consistent.'
    );
  else {
    console.error(failures.map((message) => `- ${message}`).join('\n'));
    process.exitCode = 1;
  }
}
