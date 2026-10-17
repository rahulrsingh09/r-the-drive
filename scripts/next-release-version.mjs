#!/usr/bin/env node
import { calendarReleases } from './lib/calendar-versions.mjs';
import { isMain } from './lib/validation-files.mjs';

export function nextReleaseVersion(tags, month) {
  const parts = /^(\d{4})\.(0[1-9]|1[0-2])$/.exec(month);
  if (!parts) throw new Error(`Invalid Calendar Version month: ${month}`);
  const latest = calendarReleases(tags).find(
    (release) => release.year === Number(parts[1]) && release.month === Number(parts[2])
  );
  return `${month}.${latest ? latest.patch + 1 : 0}`;
}

if (isMain(import.meta.url)) {
  const values = process.argv.slice(2);
  const position = values.indexOf('--month') + 1;
  if (!position || !values[position])
    throw new Error('Usage: next-release-version.mjs --month YYYY.MM [existing tags...]');
  console.log(nextReleaseVersion(values.slice(position + 1), values[position]));
}
