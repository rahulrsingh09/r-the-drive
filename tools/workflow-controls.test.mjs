import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {readFileSync} from 'node:fs';
import test from 'node:test';
import {workflow} from './workflow-test-harness.mjs';

test('all workflow controls remain identical to the characterized baseline', () => {
  const baseline = JSON.parse(readFileSync(new URL('./workflow-controls.json', import.meta.url)));
  for (const [file, expected] of Object.entries(baseline)) {
    const document = workflow(file.replace(/\.yml$/, ''));
    for (const job of Object.values(document.jobs)) for (const step of job.steps ?? []) delete step.run;
    assert.equal(createHash('sha256').update(JSON.stringify(document)).digest('hex'), expected,
      `${file}: triggers, job dependencies, disabled gates, action pins, permissions, services or inputs changed`);
  }
});
