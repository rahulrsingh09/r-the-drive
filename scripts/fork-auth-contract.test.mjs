import assert from 'node:assert/strict';
import test from 'node:test';
import { checkAuthFixture } from './fork-auth-contract.mjs';

test('existing source reports the actual auth cases without inventing incoming coverage', () => {
  assert.deepEqual(checkAuthFixture('#[tokio::test]\nasync fn login_works() {}'), {
    tests: ['login_works'],
    enrollment: 'not-in-this-source',
  });
});

test('unadapted upstream enrollment fails with an actionable policy explanation', () => {
  assert.throws(
    () =>
      checkAuthFixture(`
    #[tokio::test]
    async fn verified_enrollment_rejects_unlisted_ids_and_preserves_existing_authority() {
      std::env::set_var("RIVIAN_GRAPHQL_GATEWAY_URL", format!("http://{address}/graphql"));
    }
  `),
    /fixture needs adaptation.*Do not skip the test/
  );
});

test('a transport-injected enrollment fixture can run without a production URL override', () => {
  assert.equal(
    checkAuthFixture(`
    #[tokio::test]
    async fn verified_enrollment_rejects_unlisted_ids_and_preserves_existing_authority() {}
  `).enrollment,
    'present'
  );
  assert.throws(() => checkAuthFixture(''), /No authentication/);
});
