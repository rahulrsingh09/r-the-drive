#!/usr/bin/env node
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { isMain, sourceRecords } from './lib/validation-files.mjs';

const roots = [
  'apps/web/src',
  'packages/ui/src',
  'packages/dashboards/src',
  'packages/hooks/src',
  'packages/themes/src',
];
const extensions = new Set(['.ts', '.tsx', '.css']);
const ignoredDirectories = new Set(['node_modules', 'dist', 'coverage', '.turbo']);

// These files own color values rather than consume them. Keep this list narrow
// and explicit so a new production raw color fails the guard by default.
const allowedOwners = new Set([
  'packages/themes/src/index.ts',
  'packages/ui/src/lib/color.ts',
  'packages/ui/src/tokens/colors.ts',
  'packages/ui/src/tokens/globals.css',
]);

const expressions = [
  /#[0-9a-f]{3,8}\b/i,
  /\brgba?\s*\(/i,
  /\b(?:text|bg|border|ring|outline|decoration|shadow)-(?:white|black|blue|indigo|sky|green|red|orange|yellow|slate|zinc|stone)(?:\b|[/-][\w.[\]-]+)/i,
];

export function colorTokenViolations(root) {
  const files = sourceRecords(root, roots, {
    extensions: [...extensions],
    ignored: ignoredDirectories,
    exclude: /\.(?:test|spec|stories)\.[jt]sx?$/,
  });
  return files.flatMap((file) => {
    if (allowedOwners.has(file.path)) return [];
    return file.source.split(/\r\n|\n|\r/).flatMap((line, index) => {
      const matched = expressions.find((expression) => expression.exec(line));
      return matched ? [`${file.path}:${index + 1}: ${line.trim()}`] : [];
    });
  });
}

if (isMain(import.meta.url)) {
  const violations = colorTokenViolations(resolve(dirname(fileURLToPath(import.meta.url)), '..'));
  if (violations.length === 0) console.log('Color-token guard passed.');
  else {
    console.error(
      ['Color-token violations found:', ...violations.map((item) => `- ${item}`)].join('\n')
    );
    process.exitCode = 1;
  }
}
