import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';

export function syncDefaults(
  root,
  entries,
  { check, createDirectories = false, perFileSuccess = false, success }
) {
  const snapshots = entries.map((entry) => {
    const destination = join(root, entry.target);
    const source = readFileSync(join(root, entry.source));
    const same = existsSync(destination) && source.equals(readFileSync(destination));
    return { ...entry, destination, source, same };
  });
  const changed = snapshots.filter((entry) => !entry.same);
  if (!check) {
    for (const entry of changed) {
      if (createDirectories) mkdirSync(dirname(entry.destination), { recursive: true });
      writeFileSync(entry.destination, entry.source);
      console.log(`synced ${entry.target}`);
    }
    return;
  }
  for (const entry of snapshots) {
    if (!entry.same) console.error(`${entry.label} drift: ${entry.target}`);
    else if (perFileSuccess) console.log(`${entry.label} are in sync`);
  }
  if (changed.length) process.exitCode = 1;
  else if (success) console.log(success);
}
