#!/usr/bin/env node

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

// Read raw scanner results: native ignores would hide new paths and stale exceptions.
export function evaluateAudit(ecosystem, report, exceptions, today, status) {
  assert(['cargo', 'pnpm'].includes(ecosystem), 'Expected cargo or pnpm.');
  assert([0, 1].includes(status), `Scanner failed with exit ${status}.`);
  assert(!report.error, 'Scanner returned an error.');
  let findings;
  if (ecosystem === 'cargo') {
    assert(Array.isArray(report.vulnerabilities?.list), 'Missing Cargo vulnerability list.');
    assert(report.warnings && typeof report.warnings === 'object', 'Missing Cargo warnings.');
    assert.deepEqual(report.settings?.ignore, [], 'Cargo ignores must be empty.');
    assert.equal(report.vulnerabilities.count, report.vulnerabilities.list.length);
    findings = [...report.vulnerabilities.list, ...Object.values(report.warnings).flat()].map(
      (entry) => ({
        id: entry.advisory?.id,
        package: entry.package?.name,
        version: entry.package?.version,
        title: entry.advisory?.title ?? entry.kind,
      })
    );
  } else {
    assert(report.advisories && typeof report.advisories === 'object', 'Missing npm advisories.');
    assert(report.metadata?.vulnerabilities, 'Missing npm vulnerability counts.');
    findings = Object.values(report.advisories).flatMap((entry) => {
      assert(entry.findings?.length, 'Advisory has no dependency paths.');
      return entry.findings.map((finding) => ({
        id: entry.github_advisory_id,
        package: entry.module_name,
        version: finding.version,
        paths: finding.paths,
        title: entry.title,
      }));
    });
  }
  assert(status === 0 || findings.length > 0, 'Scanner failed without findings.');
  const failures = [];
  const accepted = [];
  const relevant = exceptions.filter((entry) => entry.ecosystem === ecosystem);
  for (const entry of relevant) {
    assert(
      [
        'id',
        'package',
        'version',
        'owner',
        'reviewed',
        'expires',
        'source',
        'reason',
        'removal',
      ].every((key) => typeof entry[key] === 'string' && entry[key].length > 0),
      'Incomplete audit exception.'
    );
    for (const date of [entry.reviewed, entry.expires]) {
      assert(
        /^\d{4}-\d{2}-\d{2}$/.test(date) && new Date(date).toISOString().slice(0, 10) === date,
        'Invalid exception date.'
      );
    }
    assert(
      entry.reviewed <= today && entry.reviewed <= entry.expires,
      'Invalid exception review date.'
    );
    assert(
      Date.parse(entry.expires) - Date.parse(entry.reviewed) <= 90 * 86400000,
      'Exception exceeds 90 days.'
    );
    if (today > entry.expires) failures.push(`${entry.id}: exception expired ${entry.expires}.`);
    if (!findings.some((finding) => finding.id === entry.id)) {
      failures.push(`${entry.id}: no longer reported; remove the stale exception.`);
    }
  }
  for (const finding of findings) {
    assert(finding.id && finding.package && finding.version, 'Malformed audit finding.');
    const exception = relevant.find(
      (entry) =>
        entry.id === finding.id &&
        entry.package === finding.package &&
        entry.version === finding.version &&
        today <= entry.expires &&
        (ecosystem !== 'pnpm' ||
          (entry.pathPrefix &&
            finding.paths?.length &&
            finding.paths.every(
              (path) =>
                path.startsWith(entry.pathPrefix) &&
                (!entry.pathSuffixes || entry.pathSuffixes.some((suffix) => path.endsWith(suffix)))
            )))
    );
    if (exception)
      accepted.push(
        `${finding.id} ${finding.package}@${finding.version}: ${exception.reason} Review by ${exception.expires}.`
      );
    else failures.push(`${finding.id} ${finding.package}@${finding.version}: ${finding.title}`);
  }
  return { findings, accepted, failures };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const [ecosystem, file, status] = process.argv.slice(2);
    const exceptions = JSON.parse(
      readFileSync(new URL('../config/dependency-audit-exceptions.json', import.meta.url))
    );
    const report = JSON.parse(readFileSync(file));
    const result = evaluateAudit(
      ecosystem,
      report,
      exceptions,
      new Date().toISOString().slice(0, 10),
      Number(status)
    );
    console.log(
      `${ecosystem}: ${result.findings.length} raw finding(s), ${result.accepted.length} bounded exception(s), ${result.failures.length} policy failure(s).`
    );
    for (const line of result.accepted) console.log(`EXCEPTION: ${line}`);
    for (const line of result.failures) console.error(`FAIL: ${line}`);
    process.exitCode = result.failures.length ? 1 : 0;
  } catch (error) {
    console.error(`Dependency audit failed: ${error.message}`);
    process.exitCode = 1;
  }
}
