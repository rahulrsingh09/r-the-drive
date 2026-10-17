import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {createRequire, stripTypeScriptTypes} from 'node:module';
import path from 'node:path';
import {fileURLToPath, pathToFileURL} from 'node:url';
import test from 'node:test';

const root = fileURLToPath(new URL('../', import.meta.url));
const require = createRequire(new URL('../apps/web/package.json', import.meta.url));
const {loadConfigFromFile} = await import(pathToFileURL(require.resolve('vite')));
const load = async (name) => (await loadConfigFromFile({command: 'serve', mode: 'test'}, path.join(root, name))).config;
async function standalone(name) {
  const js = stripTypeScriptTypes(readFileSync(path.join(root, name), 'utf8'), {mode: 'transform'});
  return import(`data:text/javascript;base64,${Buffer.from(js).toString('base64')}`);
}

test('RAD review adapters expose synthetic state and retain in-memory theme updates', async () => {
  const hooks = await standalone('apps/web/.rad-review/hooks.ts');
  const state = hooks.useAuth();
  assert.equal(state, hooks.useAuth.getState());
  assert.equal(hooks.useAuth((value) => value.userId), 'fixture');
  assert.equal(await state.logout(), undefined);
  assert.equal(hooks.useAuth.subscribe(() => assert.fail('fixture must never emit'))(), undefined);
  assert.deepEqual(hooks.useResolvedVehicleSelection(), {
    effectiveVehicleId: 'fixture', vehicleSelectionReady: true, vehicles: [{id: 'fixture', model: 'R1S'}],
  });
  assert.deepEqual(hooks.useMe(), {data: {role: 'user'}});
  assert.deepEqual(hooks.useCurrentVehicleStatus(), {data: {battery_level: 78, range_miles: 240}});
  assert.deepEqual(hooks.useVehicleStatus(), {status: null, connected: true, connectionState: 'online'});
  assert.deepEqual(hooks.queryKeys.themePreferences.forUser('viewer'), ['theme', 'viewer']);
  assert.deepEqual(hooks.queryKeys.unitPreferences.current, ['units']);
  const before = await hooks.themeClient.getPreferences();
  assert.deepEqual(before, {preferences: {schemaVersion: 2, mode: 'dark', selection: {kind: 'builtin', themeId: 'rad'}}, etag: 'fixture'});
  const next = {...before.preferences, mode: 'light'};
  assert.deepEqual(await hooks.themeClient.updatePreferences(next), {preferences: next, etag: 'fixture'});
  assert.equal((await hooks.themeClient.getPreferences()).preferences, next);
  const router = await standalone('apps/web/.rad-review/router.ts');
  assert.equal(router.Link, 'a');
  assert.equal(router.useNavigate()('/unused'), undefined);
});

test('Storybook alias composition preserves an existing alias array object as indexed entries', async () => {
  const {viteFinal} = await load('packages/ui/.storybook/main.ts');
  const entry = {find: 'kept', replacement: '/kept'};
  const aliases = [entry];
  const resolve = {alias: aliases};
  const input = {resolve};
  const result = await viteFinal(input);
  assert.equal(result, input);
  assert.equal(result.resolve, resolve);
  assert.equal(result.resolve.alias[0], entry);
  assert.equal(result.resolve.alias['@panda/ui'], path.join(root, 'packages/ui/src'));
  assert.deepEqual(aliases, [entry]);
});

test('chunk classifier handles nested vendor paths, prefixes, query suffixes and Windows separators', async () => {
  const select = (await load('apps/web/vite.config.ts')).build.rollupOptions.output.manualChunks;
  const modules = [
    ['react', 'react-vendor'], ['react-dom', 'react-vendor'],
    ['@tanstack/react-router', 'router'], ['@tanstack/react-query', 'query'],
  ];
  for (const [first, firstGroup] of modules) for (const [last, lastGroup] of modules) {
    const priority = ['react-vendor', 'router', 'query'];
    const expected = priority[Math.min(priority.indexOf(firstGroup), priority.indexOf(lastGroup))];
    for (const prefix of ['', '/workspace/', 'virtual:']) {
      assert.equal(select(`${prefix}node_modules/${first}/node_modules/${last}/file.ts?x`), expected);
    }
  }
  for (const name of ['node_modules\\react\\index.js', 'node_modules/React/index.js', 'react/index.js', 'node_modules/react-extra/file.js']) {
    assert.equal(select(name), undefined);
  }
});

test('docs edit URL adapter preserves doc path text and site safety policies', async () => {
  const config = await load('apps/docs/docusaurus.config.ts');
  const docs = config.presets[0][1].docs;
  for (const docPath of ['index.md', 'guides/secure-deployment.md', 'folder/with space.md']) {
    assert.equal(docs.editUrl({docPath}), `https://github.com/your-org/r-the-drive/edit/mainline/docs/${docPath}`);
  }
  for (const key of ['onBrokenLinks', 'onBrokenAnchors', 'onDuplicateRoutes']) assert.equal(config[key], 'throw');
  assert.equal(config.baseUrl, '/');
  assert.equal(config.trailingSlash, true);
  assert.equal(config.markdown.hooks.onBrokenMarkdownImages, 'throw');
});

test('Playwright config binds loopback, rejects server reuse and preserves environment conversion', async () => {
  const previous = {CI: process.env.CI, E2E_PORT: process.env.E2E_PORT};
  try {
    for (const port of [undefined, '', '0', '52789']) for (const ci of ['', 'true']) {
      process.env.CI = ci;
      if (port === undefined) delete process.env.E2E_PORT;
      else process.env.E2E_PORT = port;
      const config = await load('apps/web/playwright.config.ts');
      const number = Number(port ?? 5199);
      assert.equal(config.use.baseURL, `http://127.0.0.1:${number}`);
      assert.equal(config.webServer.url, config.use.baseURL);
      assert.equal(config.webServer.command, `pnpm exec vite --host 127.0.0.1 --port ${number} --strictPort`);
      assert.equal(config.webServer.reuseExistingServer, false);
      assert.equal(config.webServer.timeout, 120000);
      assert.deepEqual(config.reporter, ci ? [['github'], ['html', {outputFolder: 'playwright-report', open: 'never'}]] : 'list');
    }
  } finally {
    for (const [key, value] of Object.entries(previous)) {
      if (value === undefined) delete process.env[key];
      else process.env[key] = value;
    }
  }
});
