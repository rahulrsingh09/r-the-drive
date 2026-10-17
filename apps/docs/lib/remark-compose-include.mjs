import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';

const siteDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const repoRoot = path.resolve(siteDir, '../..');
export function includeComposeFile(sourceFile) {
  const forbidden = !sourceFile || [path.isAbsolute(sourceFile), sourceFile.includes('..')].some(Boolean);
  if (forbidden) {
    throw new Error(`Invalid documentation include path: ${sourceFile ?? '(missing)'}`);
  }

  const location = path.resolve(repoRoot, sourceFile);
  const insideRoot = path.relative(repoRoot, location);
  if (!insideRoot || insideRoot.startsWith(`..${path.sep}`) || path.isAbsolute(insideRoot) || !fs.existsSync(location)) {
    throw new Error(`Documentation include target does not exist: ${sourceFile}`);
  }

  const text = fs.readFileSync(location, 'utf8');
  return text.trimEnd();
}

export function remarkComposeInclude() {
  return (tree) => {
    if (!Array.isArray(tree.children)) return;
    const frames = [{node: tree, index: 0, children: []}];
    while (frames.length > 0) {
      const frame = frames[frames.length - 1];
      if (frame.index === frame.node.children.length) {
        frame.node.children = frame.children;
        frames.pop();
        continue;
      }

      const index = frame.index++;
      if (!(index in frame.node.children)) continue;
      const child = frame.node.children[index];
      if (child.type === 'code' && child.lang === 'compose-include') {
        frame.children.push({type: 'code', lang: 'yaml', value: includeComposeFile(child.value.trim())});
        continue;
      }
      frame.children.push(child);
      if (Array.isArray(child.children)) {
        frames.push({node: child, index: 0, children: []});
      }
    }
  };
}
