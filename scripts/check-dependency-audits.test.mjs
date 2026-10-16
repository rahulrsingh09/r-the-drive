import assert from 'node:assert/strict';
import { test } from 'node:test';
import { evaluateAudit } from './check-dependency-audits.mjs';

const exception = {
  ecosystem: 'pnpm',
  id: 'GHSA-test',
  package: 'braces',
  version: '3.0.3',
  owner: 'Release maintainer',
  reviewed: '2026-10-05',
  expires: '2026-11-05',
  source: 'https://github.com/advisories/GHSA-test',
  reason: 'Build tooling only.',
  removal: 'Remove on fix.',
  pathPrefix: 'apps__docs>',
};
function npmReport(overrides = {}) {
  return {
    advisories: {
      1: {
        github_advisory_id: 'GHSA-test',
        module_name: 'braces',
        title: 'Stack exhaustion',
        findings: [{ version: '3.0.3', paths: ['apps__docs>micromatch>braces'], ...overrides }],
      },
    },
    metadata: { vulnerabilities: { high: 1 } },
  };
}
const check = (report = npmReport(), today = '2026-10-05', status = 1) =>
  evaluateAudit('pnpm', report, [exception], today, status);

test('a scoped exception preserves the finding and its explanation', () => {
  const result = check();
  assert.equal(result.findings.length, 1);
  assert.equal(result.accepted.length, 1);
  assert.deepEqual(result.failures, []);
  assert.match(result.accepted[0], /Review by 2026-11-05/);
});

test('expiry permits the review date but rejects the following day', () => {
  assert.deepEqual(check(npmReport(), '2026-11-05').failures, []);
  assert.match(check(npmReport(), '2026-11-06').failures.join('\n'), /expired/);
});

test('new package versions and paths invalidate the exception', () => {
  for (const override of [
    { version: '3.0.4' },
    { paths: [] },
    { paths: ['apps__docs>braces', 'apps__web>braces'] },
  ]) {
    assert(check(npmReport(override)).failures.length > 0);
  }
});

test('path suffix restrictions reject new uses under the same workspace', () => {
  const exceptions = [{ ...exception, pathSuffixes: ['>micromatch>braces'] }];
  const result = evaluateAudit(
    'pnpm',
    npmReport({ paths: ['apps__docs>server>braces'] }),
    exceptions,
    '2026-10-05',
    1
  );
  assert.equal(result.accepted.length, 0);
});

test('invalid or unbounded exception dates fail closed', () => {
  for (const expires of ['2026-99-99', '2027-11-05']) {
    assert.throws(() =>
      evaluateAudit('pnpm', npmReport(), [{ ...exception, expires }], '2026-10-05', 1)
    );
  }
});

test('removed advisories force removal of stale exceptions', () => {
  const result = check({ advisories: {}, metadata: { vulnerabilities: {} } }, '2026-10-05', 0);
  assert.match(result.failures[0], /stale exception/);
});

test('unknown advisories remain blocking', () => {
  const report = npmReport();
  report.advisories[2] = { ...report.advisories[1], github_advisory_id: 'GHSA-new' };
  assert.match(check(report).failures[0], /GHSA-new/);
});

test('scanner and malformed-output errors fail closed', () => {
  for (const report of [{}, { error: 'network' }, { advisories: {}, metadata: {} }]) {
    assert.throws(() => check(report));
  }
  assert.throws(() => check(npmReport(), '2026-10-05', 2), /Scanner failed/);
  assert.throws(() => check(npmReport(), '2026-10-05', NaN), /Scanner failed/);
  assert.throws(
    () => check({ advisories: {}, metadata: { vulnerabilities: {} } }),
    /without findings/
  );
});

test('Cargo vulnerabilities and informational warnings both remain blocking', () => {
  const finding = {
    advisory: { id: 'RUSTSEC-test', title: 'Warning' },
    package: { name: 'crate', version: '1.0.0' },
  };
  const report = {
    settings: { ignore: [] },
    vulnerabilities: { count: 1, list: [finding] },
    warnings: { unmaintained: [{ ...finding, advisory: { id: 'RUSTSEC-other' } }] },
  };
  const result = evaluateAudit('cargo', report, [], '2026-10-05', 1);
  assert.equal(result.failures.length, 2);
  report.settings.ignore.push('RUSTSEC-test');
  assert.throws(() => evaluateAudit('cargo', report, [], '2026-10-05', 1), /ignores must be empty/);
});

test('Cargo exceptions match exact versions and expire', () => {
  const report = {
    settings: { ignore: [] },
    warnings: {},
    vulnerabilities: {
      count: 1,
      list: [
        {
          advisory: { id: 'RUSTSEC-test' },
          package: { name: 'rsa', version: '0.9.10' },
        },
      ],
    },
  };
  const exceptions = [
    { ...exception, ecosystem: 'cargo', id: 'RUSTSEC-test', package: 'rsa', version: '0.9.10' },
  ];
  assert.equal(evaluateAudit('cargo', report, exceptions, '2026-10-05', 1).accepted.length, 1);
  report.vulnerabilities.list[0].package.version = '0.9.11';
  assert.equal(evaluateAudit('cargo', report, exceptions, '2026-10-05', 1).accepted.length, 0);
});
