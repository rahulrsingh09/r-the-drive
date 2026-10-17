#!/usr/bin/env node
import { spawnSync } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { isMain, sourceRecords } from './lib/validation-files.mjs';

const sourceExtensions = ['.ts', '.tsx'];
const ignored = new Set(['node_modules', 'dist', 'coverage', '.turbo', '__tests__', 'test']);
const packageRoots = new Map([
  ['@panda/ui', 'packages/ui/src'],
  ['@panda/hooks', 'packages/hooks/src'],
  ['@panda/dashboards', 'packages/dashboards/src'],
  ['@panda/types', 'packages/types/src'],
]);
const transport =
  /\bfetch\s*\(|new\s+(?:XMLHttpRequest|WebSocket|EventSource)\b|\baxios\s*(?:\.|\()/;
const records = (root, paths, extensions = sourceExtensions) =>
  sourceRecords(root, paths, { extensions, ignored, exclude: /\.(test|stories)\.[jt]sx?$/ });
const size = (source) => ({
  lines:
    (source.match(/\r\n|\n|\r/g)?.length ?? 0) +
    Number(source.length > 0 && !/[\r\n]$/.test(source)),
  bytes: Buffer.byteLength(source.replace(/\r\n?/g, '\n')),
});

function patternFailures(root, patterns = {}) {
  const failures = [];
  for (const [name, rule] of Object.entries(patterns)) {
    if (rule.enabled === false) continue;
    const flags = rule.flags ?? 'm';
    const allowed = new Set(rule.allow ?? []);
    let total = 0;
    const offenders = [];
    for (const file of records(root, rule.directories ?? [], rule.extensions)) {
      const expression = new RegExp(rule.pattern, flags.includes('g') ? flags : flags + 'g');
      const count = [...file.source.matchAll(expression)].length;
      total += count;
      if (count && !allowed.has(file.path)) offenders.push(file.path);
    }
    if (offenders.length) failures.push(`${rule.message ?? name} (${offenders.join(', ')})`);
    if (rule.maxMatches !== undefined && total > rule.maxMatches)
      failures.push(`${rule.message ?? name} has ${total} matches; budget is ${rule.maxMatches}`);
  }
  return failures;
}

function budgetFailures(root, budgets) {
  const failures = [];
  for (const [path, limit] of Object.entries(budgets.hotspots ?? {})) {
    if (!existsSync(join(root, path))) {
      failures.push(`Architecture hotspot is missing from its budget: ${path}`);
      continue;
    }
    const measured = size(readFileSync(join(root, path), 'utf8'));
    for (const [key, setting] of [
      ['lines', 'maxLines'],
      ['bytes', 'maxBytes'],
    ]) {
      if (limit[setting] !== undefined && measured[key] > limit[setting])
        failures.push(`${path} grew to ${measured[key]} ${key}; budget is ${limit[setting]}`);
    }
  }
  const orchestration = budgets.orchestration ?? {};
  if (orchestration.maxLines && orchestration.roots?.length) {
    const exceptions = new Set(Object.keys(orchestration.existingExceptions ?? {}));
    const offenders = records(
      root,
      orchestration.roots,
      orchestration.extensions ?? ['.ts', '.tsx', '.rs']
    ).filter(
      (file) => !exceptions.has(file.path) && size(file.source).lines > orchestration.maxLines
    );
    if (offenders.length)
      failures.push(
        `New orchestration files over ${orchestration.maxLines} lines require an architecture-budget exception: ${offenders.map((file) => file.path).join(', ')}`
      );
  }
  return failures;
}

function importTarget(root, file, specifier) {
  let base;
  if (specifier.startsWith('.')) base = resolve(dirname(file), specifier);
  else {
    const alias = [...packageRoots.keys()].find(
      (name) => specifier === name || specifier.startsWith(name + '/')
    );
    if (!alias) return;
    base = join(root, packageRoots.get(alias), specifier.slice(alias.length).replace(/^\//, ''));
  }
  const candidates = [
    ...sourceExtensions.map((extension) => base + extension),
    ...sourceExtensions.map((extension) => join(base, 'index' + extension)),
  ];
  return candidates.find((candidate) => existsSync(candidate));
}

export function findCircularDependencies(root) {
  const files = records(root, ['apps/web/src', ...packageRoots.values()]);
  const graph = new Map(files.map((file) => [file.file, []]));
  for (const file of files) {
    const edges = graph.get(file.file);
    for (const match of file.source.matchAll(
      /(?:import|export)\s+(?:type\s+)?(?:[^'";]*?\s+from\s+)?['"]([^'"]+)['"]/g
    )) {
      const target = importTarget(root, file.file, match[1]);
      if (graph.has(target)) edges.push(target);
    }
  }
  const complete = new Set();
  const cycles = [];
  for (const start of graph.keys()) {
    if (complete.has(start)) continue;
    const frames = [{ file: start, edge: 0 }];
    const active = new Map([[start, 0]]);
    while (frames.length) {
      const frame = frames.at(-1);
      const edges = graph.get(frame.file);
      if (frame.edge === edges.length) {
        complete.add(frame.file);
        active.delete(frame.file);
        frames.pop();
        continue;
      }
      const target = edges[frame.edge++];
      if (active.has(target)) {
        cycles.push(
          [...frames.slice(active.get(target)).map((entry) => entry.file), target].map((file) =>
            relative(root, file).replaceAll('\\', '/')
          )
        );
      } else if (!complete.has(target)) {
        active.set(target, frames.length);
        frames.push({ file: target, edge: 0 });
      }
    }
  }
  return cycles;
}

export function architectureGuardFailures(root) {
  let budgets = {};
  const failures = [];
  const config = join(root, 'config/architecture-budgets.json');
  try {
    if (!existsSync(config))
      throw new Error(`Missing required architecture budget configuration: ${config}`);
    budgets = JSON.parse(readFileSync(config, 'utf8'));
  } catch (error) {
    failures.push(`Architecture budget configuration is invalid: ${error.message}`);
  }
  failures.push(...patternFailures(root, budgets.patterns), ...budgetFailures(root, budgets));
  for (const [directory, message] of [
    ['packages/ui/src', 'Shared UI must not create network transports'],
    ['apps/web/src/routes', 'Route modules must use hooks rather than raw transports'],
  ]) {
    const offenders = records(root, [directory])
      .filter((file) => transport.test(file.source))
      .map((file) => file.path);
    if (offenders.length) failures.push(`${message}: ${offenders.join(', ')}`);
  }
  const cycles = findCircularDependencies(root);
  if (cycles.length)
    failures.push(
      `Circular source dependencies are forbidden:\n${cycles.map((cycle) => `  - ${cycle.join(' -> ')}`).join('\n')}`
    );
  return failures;
}

export function runArchitectureGuards(root) {
  const failures = architectureGuardFailures(root);
  for (const [args, message] of [
    [['scripts/sync-dashboard-defaults.mjs', '--check'], 'Dashboard defaults are not in sync'],
    [['scripts/check-api-route-security.mjs'], 'API route security guard failed'],
  ]) {
    const result = spawnSync(process.execPath, args, { cwd: root, encoding: 'utf8' });
    if (result.error || result.status !== 0)
      failures.push(
        `${message}:\n${result.stderr?.trim() || result.error?.message || `Command failed: ${args.join(' ')}`}`
      );
  }
  return failures;
}

if (isMain(import.meta.url)) {
  const failures = runArchitectureGuards(resolve(dirname(fileURLToPath(import.meta.url)), '..'));
  if (failures.length) {
    console.error(failures.map((failure) => `- ${failure}`).join('\n'));
    process.exitCode = 1;
  } else
    console.log(
      'Architecture guards passed: budgets and duplicate-pattern ratchets are within limits, shared UI and routes own no raw transports, dashboard defaults are synced, route policy is current, and source dependencies are acyclic.'
    );
}
