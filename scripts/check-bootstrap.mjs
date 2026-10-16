import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFileSync, readdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import YAML from 'yaml';
import { readPrivateCatalog } from './lib/private-migrations.mjs';

const root = fileURLToPath(new URL('..', import.meta.url));
const read = (path) => readFileSync(new URL(`../${path}`, import.meta.url), 'utf8');
const config = JSON.parse(read('config/bootstrap.json'));
const files = execFileSync('git', ['ls-files', '-z'], { cwd: root, encoding: 'utf8' })
  .split('\0').filter(Boolean);
assert(files.length > 1000, 'The bootstrap must contain the full source tree');
for (const path of files) {
  assert(!/^(.+\/)?(?:\.env|wrangler\.json|age_key\.txt|jwt_private\.pem)$/.test(path),
    `Private configuration must not be tracked: ${path}`);
  assert(!path.endsWith('.xcf'), `Retired layered artwork must not be tracked: ${path}`);
}

const ledger = JSON.parse(read('config/native-release-catalog.json'));
const catalog = readPrivateCatalog(root);
assert.deepEqual(ledger, catalog.map(({ identity }) => ({
  version: identity.version,
  checksum: identity.checksum_sha384,
  success: true,
})), 'Release catalog must match the renamed migration bytes');
for (const file of readdirSync(new URL('../apps/api/.sqlx/', import.meta.url))) {
  if (!file.endsWith('.json')) continue;
  const query = JSON.parse(read(`apps/api/.sqlx/${file}`));
  const hash = createHash('sha256').update(query.query).digest('hex');
  assert.equal(query.hash, hash, `SQLx query digest: ${file}`);
  assert.equal(file, `query-${hash}.json`, `SQLx query filename: ${file}`);
}

for (const file of readdirSync(new URL('../.github/workflows/', import.meta.url))) {
  if (!file.endsWith('.yml')) continue;
  const workflow = YAML.parse(read(`.github/workflows/${file}`), { uniqueKeys: true });
  assert(workflow.on && workflow.jobs, `Invalid workflow: ${file}`);
  if (['fork-ci.yml', 'security.yml'].includes(file)) continue;
  for (const [name, job] of Object.entries(workflow.jobs)) {
    assert(job.if?.includes("vars.PANDA_ENABLE_RELEASE_AUTOMATION == 'true'"),
      `Unconfigured automation must be gated: ${file}/${name}`);
  }
}
const ci = YAML.parse(read('.github/workflows/fork-ci.yml'));
assert.equal(ci.permissions.contents, 'read');
assert(!read('.github/workflows/fork-ci.yml').includes('repository.private == false'),
  'Validation must run in private repositories');
assert.equal(config.freshDatabaseRequired, true);
assert.equal(config.deploymentConfigured, false);
console.log(`Bootstrap verified: ${files.length} files, ${catalog.length} migrations, isolated automation.`);
