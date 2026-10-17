import { processRegistryFiles, repositoryRoot } from './registryFiles.mjs';

const targets = await processRegistryFiles(repositoryRoot, 'write');
for (const target of targets) console.log(`Generated ${target}`);
