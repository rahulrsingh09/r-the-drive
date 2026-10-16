#!/usr/bin/env node
/**
 * Thin Komodo Procedure entrypoint. Komodo supplies non-secret paths and
 * immutable image/provenance values as environment variables; the harness
 * remains the policy owner and never receives credentials on its command line.
 */
import { main } from './dev-harness.mjs';

const value = (name) => process.env[name]?.trim();
const args = [];
const packagePath = value('PANDA_HARNESS_PACKAGE');
if (packagePath) args.push('--package', packagePath);
else {
  const packageDir = value('PANDA_HARNESS_PACKAGE_DIR');
  if (!packageDir) {
    console.error('dev-harness-procedure: PANDA_HARNESS_PACKAGE or PANDA_HARNESS_PACKAGE_DIR is required.');
    process.exitCode = 1;
    process.exit();
  }
  args.push('--latest-package-dir', packageDir);
}

const required = {
  PANDA_HARNESS_BASELINE_IMAGE: '--baseline-image',
  PANDA_HARNESS_DEV_IMAGE: '--dev-image',
  PANDA_HARNESS_PRODUCTION_ROOT: '--production-root',
  PANDA_HARNESS_PRODUCTION_PORT: '--production-port',
  PANDA_HARNESS_TEST_ROOT: '--test-root',
  PANDA_HARNESS_ENV_FILE: '--env-file',
  PANDA_HARNESS_COMPOSE_FILE: '--compose-file',
  PANDA_HARNESS_PROJECT: '--project',
};
for (const [name, flag] of Object.entries(required)) {
  const current = value(name);
  if (!current) {
    console.error(`dev-harness-procedure: ${name} is required.`);
    process.exitCode = 1;
    process.exit();
  }
  args.push(flag, current);
}

const optional = {
  PANDA_HARNESS_SHA256: '--sha256',
  PANDA_HARNESS_SOURCE_SHA: '--source-sha',
  PANDA_HARNESS_PRERELEASE_TAG: '--prerelease-tag',
  PANDA_HARNESS_COMPOSE_SOURCE_SHA: '--compose-source-sha',
  PANDA_HARNESS_KOMODO_REVISION: '--komodo-revision',
  PANDA_HARNESS_HEALTH_HOST: '--health-host',
  PANDA_HARNESS_PORT: '--port',
};
for (const [name, flag] of Object.entries(optional)) {
  const current = value(name);
  if (current) args.push(flag, current);
}

if (value('PANDA_HARNESS_RESET_TEST_STORAGE')?.toLowerCase() === 'true') args.push('--reset-test-storage');
if (value('PANDA_HARNESS_PLAN_ONLY')?.toLowerCase() === 'true') args.push('--plan');
else args.push('--execute');

await main(args);
