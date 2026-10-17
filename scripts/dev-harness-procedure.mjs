#!/usr/bin/env node
import { main } from './dev-harness.mjs';
import { direct } from './operational-runtime.mjs';

export function procedureArguments(env) {
  const values = Object.fromEntries(Object.entries(env).map(([key, value]) => [key, value?.trim()]));
  const fields = [
    ['BASELINE_IMAGE', 'baseline-image', true], ['DEV_IMAGE', 'dev-image', true],
    ['PRODUCTION_ROOT', 'production-root', true], ['PRODUCTION_PORT', 'production-port', true],
    ['TEST_ROOT', 'test-root', true], ['ENV_FILE', 'env-file', true],
    ['COMPOSE_FILE', 'compose-file', true], ['PROJECT', 'project', true],
    ['SHA256', 'sha256'], ['SOURCE_SHA', 'source-sha'], ['PRERELEASE_TAG', 'prerelease-tag'],
    ['COMPOSE_SOURCE_SHA', 'compose-source-sha'], ['KOMODO_REVISION', 'komodo-revision'],
    ['HEALTH_HOST', 'health-host'], ['PORT', 'port'],
  ];
  const args = values.PANDA_HARNESS_PACKAGE ? ['--package', values.PANDA_HARNESS_PACKAGE]
    : ['--latest-package-dir', values.PANDA_HARNESS_PACKAGE_DIR];
  if (!args[1]) throw new Error('PANDA_HARNESS_PACKAGE or PANDA_HARNESS_PACKAGE_DIR is required.');
  for (const [suffix, flag, required] of fields) {
    const name = 'PANDA_HARNESS_' + suffix;
    if (values[name]) args.push('--' + flag, values[name]);
    else if (required) throw new Error(`${name} is required.`);
  }
  for (const [name, flag] of [['RESET_TEST_STORAGE', '--reset-test-storage'], ['PLAN_ONLY', '--plan']])
    if (values['PANDA_HARNESS_' + name]?.toLowerCase() === 'true') args.push(flag);
  if (!args.includes('--plan')) args.push('--execute');
  return args;
}
if (direct(import.meta.url)) {
  try { await main(procedureArguments(process.env)); }
  catch (error) { console.error(`dev-harness-procedure: ${error.message}`); process.exitCode = 1; }
}
