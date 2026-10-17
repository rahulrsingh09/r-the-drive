import { readFileSync } from 'node:fs';
import { direct } from './operational-runtime.mjs';

export function checkAuthFixture(source) {
  const tests = [];
  const declaration = /#\[tokio::test[^\]]*\]\s*async fn (\w+)/g;
  let match;
  while ((match = declaration.exec(source)) !== null) tests.push(match[1]);
  if (tests.length === 0) throw new Error('No authentication integration tests found');
  const enrollmentName = 'verified_enrollment_rejects_unlisted_ids_and_preserves_existing_authority';
  const enrollment = tests.indexOf(enrollmentName) >= 0;
  if (enrollment) {
    const override = /std::env::set_var\(\s*"RIVIAN_GRAPHQL_GATEWAY_URL"/;
    if (override.exec(source)) throw new Error(
      'Upstream enrollment fixture needs adaptation: its process-wide Rivian gateway override ' +
      'conflicts with the fork URL policy. Provide an in-process test transport at the verified ' +
      'enrollment boundary while preserving production URL validation and authority assertions. ' +
      'Do not skip the test, allow HTTP/loopback in production, or use real Rivian credentials.');
  }
  return { tests, enrollment: enrollment ? 'present' : 'not-in-this-source' };
}
export function main() {
  try {
    console.log(JSON.stringify(checkAuthFixture(readFileSync('apps/api/tests/auth_integration.rs', 'utf8')), null, 2));
  } catch (error) {
    process.exitCode = 1;
    console.error(error.message);
  }
}
if (direct(import.meta.url)) main();
