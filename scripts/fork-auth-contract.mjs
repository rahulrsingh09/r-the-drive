import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

export function checkAuthFixture(source) {
  const tests = [...source.matchAll(/#\[tokio::test[^\]]*\]\s*async fn (\w+)/g)].map(
    (match) => match[1]
  );
  const enrollment = tests.includes(
    'verified_enrollment_rejects_unlisted_ids_and_preserves_existing_authority'
  );
  if (enrollment && /std::env::set_var\(\s*"RIVIAN_GRAPHQL_GATEWAY_URL"/.test(source)) {
    throw new Error(
      'Upstream enrollment fixture needs adaptation: its process-wide Rivian gateway override ' +
        'conflicts with the fork URL policy. Provide an in-process test transport at the verified ' +
        'enrollment boundary while preserving production URL validation and authority assertions. ' +
        'Do not skip the test, allow HTTP/loopback in production, or use real Rivian credentials.'
    );
  }
  if (tests.length === 0) throw new Error('No authentication integration tests found');
  return { tests, enrollment: enrollment ? 'present' : 'not-in-this-source' };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const result = checkAuthFixture(readFileSync('apps/api/tests/auth_integration.rs', 'utf8'));
    console.log(JSON.stringify(result, null, 2));
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
