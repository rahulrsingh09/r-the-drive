import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';

const siteDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const repoRoot = path.resolve(siteDir, '../..');
const docsRoot = path.join(repoRoot, 'docs');
const githubRoot = 'https://github.com/your-org/r-the-drive';

function resolveReference(sourceFilePath, url) {
  const candidates = [siteDir, repoRoot].map((directory) => path.resolve(directory, sourceFilePath));
  const source = candidates.find((candidate) => fs.existsSync(candidate)) ?? candidates[1];
  const pieces = /^([^?#]*)(\?[^#]*)?(#.*)?$/.exec(url);
  const [target, query, fragment] = pieces ? pieces.slice(1) : [url];
  const location = path.resolve(path.dirname(source), decodeURIComponent(target));
  const suffix = [query, fragment].join('');
  if (!fs.existsSync(location)) {
    throw new Error(`Broken documentation link in ${sourceFilePath}: ${url}`);
  }
  const sectionPath = path.relative(docsRoot, location);
  return {
    location,
    suffix,
    document: !sectionPath.startsWith('..') && !path.isAbsolute(sectionPath),
  };
}

export function rewriteRepositoryLink({sourceFilePath, url}) {
  if (!url || /^(?:[a-z][a-z\d+.-]*:|\/|#)/i.test(url)) return url;

  const reference = resolveReference(sourceFilePath, url);
  if (reference.document) return url;
  const pathSegments = path.relative(repoRoot, reference.location).split(path.sep);
  const kind = fs.statSync(reference.location).isDirectory() ? 'tree' : 'blob';
  const repositoryPath = [githubRoot, kind, 'mainline', ...pathSegments.map(encodeURIComponent)].join('/');
  return repositoryPath + reference.suffix;
}

export function remarkRepositoryLinks() {
  return (tree, file) => {
    const pending = [tree];
    while (pending.length > 0) {
      const node = pending.pop();
      if (node.type === 'link' && typeof node.url === 'string') {
        node.url = rewriteRepositoryLink({sourceFilePath: file.path, url: node.url});
      }
      if (Array.isArray(node.children)) {
        for (let index = node.children.length - 1; index >= 0; index--) {
          pending.push(node.children[index]);
        }
      }
    }
  };
}
