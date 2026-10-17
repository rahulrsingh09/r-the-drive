#!/usr/bin/env node
import { spawnSync } from 'node:child_process';
import { calendarReleases } from './lib/calendar-versions.mjs';
import { isMain } from './lib/validation-files.mjs';

export function latestRelease(allTags) {
  return calendarReleases(allTags)[0]?.tag ?? null;
}

export function resolveBuildVersion(allTags, headTags = [], commitsSince = null) {
  const release = latestRelease(allTags);
  const state =
    release === null ? 'missing' : headTags.includes(release) ? 'tagged' : 'development';
  switch (state) {
    case 'missing':
      return 'unknown';
    case 'tagged':
      return release;
    default:
      return [release + '+dev', ...(commitsSince === null ? [] : [commitsSince])].join('.');
  }
}

export function resolveBuildVersionFromGit({ candidate = false } = {}) {
  const readGit = (...args) => {
    const result = spawnSync('git', args, {
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'ignore'],
    });
    if (result.error || result.status !== 0) throw new Error('Git version metadata unavailable');
    return result.stdout
      .trim()
      .split(/\r?\n/)
      .map((line) => line.trim())
      .filter(Boolean);
  };
  try {
    const releases = readGit('tag', '--list');
    const latest = latestRelease(releases);
    const distance =
      candidate && latest ? Number(readGit('rev-list', '--count', `${latest}..HEAD`)[0]) : null;
    const atHead = readGit('tag', '--points-at', 'HEAD');
    return resolveBuildVersion(releases, atHead, distance);
  } catch {
    return 'unknown';
  }
}

if (isMain(import.meta.url))
  console.log(
    resolveBuildVersionFromGit({ candidate: process.argv.slice(2).includes('--candidate') })
  );
