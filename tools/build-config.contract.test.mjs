import assert from 'node:assert/strict';
import {EventEmitter} from 'node:events';
import {readFile} from 'node:fs/promises';
import {createRequire, stripTypeScriptTypes} from 'node:module';
import path from 'node:path';
import {fileURLToPath, pathToFileURL} from 'node:url';
import test from 'node:test';

const root = fileURLToPath(new URL('../', import.meta.url));
const require = createRequire(new URL('../apps/web/package.json', import.meta.url));
const {loadConfigFromFile} = await import(pathToFileURL(require.resolve('vite')));
const load = async (file) => (await loadConfigFromFile(
  {command: 'serve', mode: 'test'}, path.join(root, file),
)).config;

test('development proxy defaults and environment precedence remain stable', async () => {
  const keys = ['VITE_API_URL', 'VITE_PANDA_API_BASE_URL', 'VITE_RESTORE_AGENT_URL'];
  const original = Object.fromEntries(keys.map((key) => [key, process.env[key]]));
  try {
    for (const env of [
      {},
      {VITE_PANDA_API_BASE_URL: 'http://legacy.example.test'},
      {VITE_API_URL: 'http://primary.example.test', VITE_PANDA_API_BASE_URL: 'http://legacy.example.test'},
      {VITE_API_URL: '', VITE_RESTORE_AGENT_URL: ''},
      {VITE_RESTORE_AGENT_URL: 'http://restore.example.test'},
    ]) {
      for (const key of keys) {
        if (Object.hasOwn(env, key)) process.env[key] = env[key];
        else delete process.env[key];
      }
      const {server} = await load('apps/web/vite.config.ts');
      const target = env.VITE_API_URL ?? env.VITE_PANDA_API_BASE_URL ?? 'http://localhost:3001';
      assert.equal(server.proxy['/v1'].target, target);
      assert.equal(server.proxy['/v2'].target, target);
      assert.equal(server.proxy['/v1/restore-runtime'].target, env.VITE_RESTORE_AGENT_URL ?? 'http://127.0.0.1:3002');
      assert.equal(server.proxy['/v1'].ws, true);
      for (const proxy of Object.values(server.proxy)) assert.equal(proxy.changeOrigin, true);
    }
  } finally {
    for (const key of keys) {
      if (original[key] === undefined) delete process.env[key];
      else process.env[key] = original[key];
    }
  }
});

test('development proxy suppresses only connection resets and retains error objects', async (t) => {
  const config = await load('apps/web/vite.config.ts');
  const proxy = new EventEmitter();
  config.server.proxy['/v1'].configure(proxy);
  const log = t.mock.method(console, 'error', () => {});
  const reset = Object.assign(new Error('reset'), {code: 'ECONNRESET'});
  proxy.emit('error', reset);
  assert.equal(log.mock.callCount(), 0);
  for (const code of [undefined, '', 'ECONNREFUSED', 'ETIMEDOUT', 'econnreset']) {
    const error = Object.assign(new Error('connection failure'), {code});
    proxy.emit('error', error);
    assert.deepEqual(log.mock.calls.at(-1).arguments, ['[vite] ws proxy error:', error]);
  }
  assert.equal(log.mock.callCount(), 5);
});

test('vendor chunk selection preserves module boundaries and precedence', async () => {
  const select = (await load('apps/web/vite.config.ts')).build.rollupOptions.output.manualChunks;
  const vendors = [
    ['react', 'react-vendor'],
    ['react-dom', 'react-vendor'],
    ['@tanstack/react-router', 'router'],
    ['@tanstack/react-query', 'query'],
  ];
  for (const [name, expected] of vendors) {
    assert.equal(select(`/workspace/node_modules/${name}/index.js`), expected);
    assert.equal(select(`node_modules/${name}/dist/file.js?version=1`), expected);
    assert.equal(select(`/workspace/node_modules/${name}-extra/index.js`), undefined);
    assert.equal(select(`/workspace/node_modules/${name}`), undefined);
  }
  assert.equal(select('/node_modules/@tanstack/react-query/node_modules/react/index.js'), 'react-vendor');
  assert.equal(select('/workspace/src/react/index.tsx'), undefined);
});

test('storybook alias binding preserves caller objects and existing aliases', async () => {
  const {viteFinal} = await load('packages/ui/.storybook/main.ts');
  for (const resolve of [undefined, {}, {alias: {'old': '/old', '@panda/ui': '/obsolete'}}]) {
    const config = {resolve, extra: {kept: true}};
    const result = await viteFinal(config);
    assert.equal(result, config);
    if (resolve) assert.equal(result.resolve, resolve);
    assert.equal(result.resolve.alias['@panda/ui'], path.join(root, 'packages/ui/src'));
    if (resolve?.alias.old) assert.equal(result.resolve.alias.old, '/old');
    assert.deepEqual(result.extra, {kept: true});
  }
});

test('storybook theme selection sets the document before rendering the story', async () => {
  const source = await readFile(path.join(root, 'packages/ui/.storybook/preview.ts'), 'utf8');
  const script = stripTypeScriptTypes(source.replace(/^import\s+['"][^'"]+\.css['"];?\s*$/gm, ''), {mode: 'transform'});
  const {default: preview} = await import(`data:text/javascript;base64,${Buffer.from(script).toString('base64')}`);
  const previous = Object.getOwnPropertyDescriptor(globalThis, 'document');
  const document = {documentElement: {className: 'old additional'}};
  Object.defineProperty(globalThis, 'document', {value: document, configurable: true});
  try {
    for (const value of [undefined, null, '', '#0A0A0F', '#FAFAF7', '#fafaf7']) {
      const ctx = value === undefined ? {} : {globals: {backgrounds: {value}}};
      const expected = value === '#FAFAF7' ? 'light' : 'dark';
      const result = preview.decorators[0](() => {
        assert.equal(document.documentElement.className, expected);
        return {rendered: true};
      }, ctx);
      assert.deepEqual(result, {rendered: true});
    }
  } finally {
    if (previous) Object.defineProperty(globalThis, 'document', previous);
    else delete globalThis.document;
  }
});
