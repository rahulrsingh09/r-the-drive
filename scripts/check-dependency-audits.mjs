#!/usr/bin/env node
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { isMain } from './lib/validation-files.mjs';

function* scannerFindings(ecosystem, report) {
  if (ecosystem === 'pnpm') {
    assert(report.advisories && typeof report.advisories === 'object', 'Missing npm advisories.');
    assert(report.metadata?.vulnerabilities, 'Missing npm vulnerability counts.');
    for (const advisory of Object.values(report.advisories)) {
      assert(advisory.findings?.length, 'Advisory has no dependency paths.');
      for (const dependency of advisory.findings)
        yield {
          id: advisory.github_advisory_id,
          package: advisory.module_name,
          version: dependency.version,
          paths: dependency.paths,
          title: advisory.title,
        };
    }
    return;
  }
  const list = report.vulnerabilities?.list;
  assert(Array.isArray(list), 'Missing Cargo vulnerability list.');
  assert(report.warnings && typeof report.warnings === 'object', 'Missing Cargo warnings.');
  assert.deepEqual(report.settings?.ignore, [], 'Cargo ignores must be empty.');
  assert.equal(report.vulnerabilities.count, list.length);
  for (const group of [
    list,
    ...Object.values(report.warnings).map((warnings) => [warnings].flat()),
  ]) {
    for (const entry of group)
      yield {
        id: entry.advisory?.id,
        package: entry.package?.name,
        version: entry.package?.version,
        title: entry.advisory?.title ?? entry.kind,
      };
  }
}

function validateException(entry, today) {
  const fields = [
    'id',
    'package',
    'version',
    'owner',
    'reviewed',
    'expires',
    'source',
    'reason',
    'removal',
  ];
  for (const key of fields)
    assert(typeof entry[key] === 'string' && entry[key].length > 0, 'Incomplete audit exception.');
  const dates = [entry.reviewed, entry.expires].map((value) => {
    const timestamp = Date.parse(value);
    assert(
      /^\d{4}-\d{2}-\d{2}$/.test(value) &&
        Number.isFinite(timestamp) &&
        new Date(timestamp).toISOString().slice(0, 10) === value,
      'Invalid exception date.'
    );
    return timestamp;
  });
  assert(entry.reviewed <= today && dates[0] <= dates[1], 'Invalid exception review date.');
  assert(dates[1] - dates[0] <= 90 * 86400000, 'Exception exceeds 90 days.');
}

function covers(entry, finding, ecosystem, today) {
  if (
    entry.package !== finding.package ||
    entry.version !== finding.version ||
    today > entry.expires
  )
    return false;
  if (ecosystem === 'cargo') return true;
  if (!entry.pathPrefix || !finding.paths?.length) return false;
  for (const path of finding.paths) {
    if (!path.startsWith(entry.pathPrefix)) return false;
    if (entry.pathSuffixes && !entry.pathSuffixes.find((suffix) => path.endsWith(suffix)))
      return false;
  }
  return true;
}

export function evaluateAudit(ecosystem, report, exceptions, today, status) {
  assert(ecosystem === 'cargo' || ecosystem === 'pnpm', 'Expected cargo or pnpm.');
  assert(status === 0 || status === 1, `Scanner failed with exit ${status}.`);
  assert(!report.error, 'Scanner returned an error.');
  const findings = Array.from(scannerFindings(ecosystem, report));
  assert(status === 0 || findings.length !== 0, 'Scanner failed without findings.');
  const failures = [];
  const accepted = [];
  const byId = new Map();
  const reported = new Set(findings.map((finding) => finding.id));
  for (const entry of exceptions) {
    if (entry.ecosystem !== ecosystem) continue;
    validateException(entry, today);
    if (entry.expires < today) failures.push(`${entry.id}: exception expired ${entry.expires}.`);
    if (!reported.has(entry.id))
      failures.push(`${entry.id}: no longer reported; remove the stale exception.`);
    const group = byId.get(entry.id) ?? [];
    group.push(entry);
    byId.set(entry.id, group);
  }
  for (const finding of findings) {
    assert(finding.id && finding.package && finding.version, 'Malformed audit finding.');
    const entry = byId
      .get(finding.id)
      ?.find((candidate) => covers(candidate, finding, ecosystem, today));
    const identity = `${finding.id} ${finding.package}@${finding.version}`;
    if (!entry) failures.push(`${identity}: ${finding.title}`);
    else accepted.push(`${identity}: ${entry.reason} Review by ${entry.expires}.`);
  }
  return { findings, accepted, failures };
}

if (isMain(import.meta.url)) {
  try {
    const [ecosystem, input, code] = process.argv.slice(2);
    const result = evaluateAudit(
      ecosystem,
      JSON.parse(readFileSync(input, 'utf8')),
      JSON.parse(
        readFileSync(new URL('../config/dependency-audit-exceptions.json', import.meta.url), 'utf8')
      ),
      new Date().toISOString().slice(0, 10),
      Number(code)
    );
    const { findings, accepted, failures } = result;
    console.log(
      `${ecosystem}: ${findings.length} raw finding(s), ${accepted.length} bounded exception(s), ${failures.length} policy failure(s).`
    );
    accepted.forEach((line) => console.log(`EXCEPTION: ${line}`));
    failures.forEach((line) => console.error(`FAIL: ${line}`));
    process.exitCode = Number(failures.length !== 0);
  } catch (error) {
    console.error(`Dependency audit failed: ${error.message}`);
    process.exitCode = 1;
  }
}
