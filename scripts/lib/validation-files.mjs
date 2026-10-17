import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { extname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export function* walkFiles(directory, { ignored = new Set(), accept = () => true } = {}) {
  if (!existsSync(directory)) return;
  const pending = [directory];
  while (pending.length) {
    const file = pending.pop();
    if (!statSync(file).isDirectory()) {
      if (accept(file)) yield file;
      continue;
    }
    const entries = readdirSync(file).filter((name) => !ignored.has(name));
    for (let index = entries.length - 1; index >= 0; index -= 1)
      pending.push(join(file, entries[index]));
  }
}

export function sourceRecords(root, directories, { extensions, ignored, exclude = /$^/ }) {
  const records = [];
  for (const directory of directories) {
    for (const file of walkFiles(join(root, directory), {
      ignored,
      accept: (file) => extensions.includes(extname(file)) && !exclude.test(file),
    }))
      records.push({
        file,
        path: relative(root, file).replaceAll('\\', '/'),
        source: readFileSync(file, 'utf8'),
      });
  }
  return records;
}

export function isMain(url) {
  return Boolean(process.argv[1]) && resolve(process.argv[1]) === fileURLToPath(url);
}
