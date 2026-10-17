import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';

const siteDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const buildDir = path.join(siteDir, 'build');
const requiredTerms = ['installation', 'backup', 'dashboard', 'architecture', 'api', 'release'];
const artifacts = [
  {
    filename: 'search-index-docs-default-current.json',
    inspect(contents) {
      const indexedText = contents.toLowerCase();
      const absent = requiredTerms.find((term) => !indexedText.includes(term));
      return absent === undefined
        ? null
        : `Production documentation search index is missing the term: ${absent}`;
    },
  },
  {
    filename: 'search-index-default.json',
    inspect(contents) {
      const {documents} = JSON.parse(contents);
      return documents?.some(({sectionRoute}) => sectionRoute === '/')
        ? null
        : 'Production search index does not include the branded homepage';
    },
  },
].map((artifact) => ({...artifact, location: path.join(buildDir, artifact.filename)}));

const absentArtifact = artifacts.find(({location}) => !fs.existsSync(location));
if (absentArtifact) {
  throw new Error(`Missing production search index: ${path.relative(siteDir, absentArtifact.location)}`);
}

for (const artifact of artifacts) {
  const problem = artifact.inspect(fs.readFileSync(artifact.location, 'utf8'));
  if (problem !== null) throw new Error(problem);
}

console.log('Production search indexes contain the homepage and required documentation terms.');
