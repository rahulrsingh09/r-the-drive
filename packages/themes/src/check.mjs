import { processRegistryFiles, repositoryRoot } from './registryFiles.mjs';

await processRegistryFiles(repositoryRoot, 'check');
console.log('Theme registry check passed.');
