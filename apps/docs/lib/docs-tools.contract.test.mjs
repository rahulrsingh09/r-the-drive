import assert from 'node:assert/strict';
import {mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync} from 'node:fs';
import {tmpdir} from 'node:os';
import path from 'node:path';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import test from 'node:test';
import {includeComposeFile, remarkComposeInclude} from './remark-compose-include.mjs';
import {remarkRepositoryLinks, rewriteRepositoryLink} from './repository-links.mjs';

const root = fileURLToPath(new URL('../../../', import.meta.url));
const sourceFilePath = path.join(root, 'docs/index.md');

test('link admission preserves nonrepository references and suffix bytes', () => {
  for (const url of ['', undefined, '/local/path', '//example.test/a', '#anchor',
    'MAILTO:reader@example.test', 'custom+scheme:value', '?query#anchor']) {
    assert.equal(rewriteRepositoryLink({sourceFilePath, url}), url);
  }
  assert.equal(
    rewriteRepositoryLink({sourceFilePath, url: '../%52EADME.md?view=a%20b#part%201'}),
    'https://github.com/your-org/r-the-drive/blob/mainline/README.md?view=a%20b#part%201',
  );
  assert.throws(
    () => rewriteRepositoryLink({sourceFilePath, url: '../%XY'}),
    URIError,
  );
});

test('link source resolution supports site-relative and repository-relative inputs', () => {
  for (const source of ['../../docs/index.md', 'docs/index.md', sourceFilePath]) {
    assert.equal(
      rewriteRepositoryLink({sourceFilePath: source, url: '../compose/'}),
      'https://github.com/your-org/r-the-drive/tree/mainline/compose',
    );
    assert.equal(rewriteRepositoryLink({sourceFilePath: source, url: './privacy.md'}), './privacy.md');
  }
});

test('link traversal visits nested links in order and preserves unrelated nodes', () => {
  const first = {type: 'link', url: '../README.md', title: 'kept', children: []};
  const ignored = {type: 'image', url: '../missing-image.png'};
  const broken = {type: 'link', url: '../missing-document.md', children: []};
  const last = {type: 'link', url: '../LICENSE', children: []};
  const tree = {type: 'root', children: [{type: 'paragraph', children: [first, ignored, broken, last]}]};
  assert.throws(() => remarkRepositoryLinks()(tree, {path: sourceFilePath}), /Broken documentation link/);
  assert.equal(first.url, 'https://github.com/your-org/r-the-drive/blob/mainline/README.md');
  assert.equal(first.title, 'kept');
  assert.equal(ignored.url, '../missing-image.png');
  assert.equal(last.url, '../LICENSE');
});

test('compose includes preserve exact file content except trailing whitespace', () => {
  assert.equal(
    includeComposeFile('compose/docker-compose.yml'),
    readFileSync(path.join(root, 'compose/docker-compose.yml'), 'utf8').replace(/\s+$/, ''),
  );
  for (const source of [null, undefined, '', '/etc/hosts', '../README.md', 'a..b']) {
    assert.throws(() => includeComposeFile(source), /Invalid documentation include path/);
  }
  assert.throws(() => includeComposeFile('missing-compose-file.yml'), /Documentation include target does not exist/);
});

test('compose traversal replaces only child include nodes and leaves other nodes intact', () => {
  const include = {type: 'code', lang: 'compose-include', value: '  compose/docker-compose.yml\n', meta: 'discard'};
  const plain = {type: 'code', lang: 'text', value: 'leave me'};
  const group = {type: 'section', children: [include, plain]};
  const tree = {type: 'root', children: [group]};
  const oldChildren = group.children;
  remarkComposeInclude()(tree);
  assert.deepEqual(group.children[0], {type: 'code', lang: 'yaml', value: includeComposeFile('compose/docker-compose.yml')});
  assert.equal(group.children[1], plain);
  assert.notEqual(group.children, oldChildren);
  const rootCode = {...include};
  remarkComposeInclude()(rootCode);
  assert.deepEqual(rootCode, include);
});

test('failed compose traversal preserves assignment timing and completed nested work', () => {
  const nested = {type: 'section', children: [{type: 'code', lang: 'compose-include', value: 'compose/docker-compose.yml'}]};
  const broken = {type: 'code', lang: 'compose-include', value: '../escape.yml'};
  const children = [nested, broken];
  const tree = {type: 'root', children};
  assert.throws(() => remarkComposeInclude()(tree), /Invalid documentation include path/);
  assert.equal(tree.children, children);
  assert.equal(nested.children[0].lang, 'yaml');
  assert.equal(broken.lang, 'compose-include');
});

test('compose traversal compacts sparse child arrays while link traversal rejects them', () => {
  const leaf = {type: 'text', value: 'kept'};
  const children = new Array(3);
  children[1] = leaf;
  const tree = {type: 'root', children};
  remarkComposeInclude()(tree);
  assert.deepEqual(tree.children, [leaf]);
  assert.equal(tree.children[0], leaf);
  assert.throws(
    () => remarkRepositoryLinks()({type: 'root', children}, {path: sourceFilePath}),
    TypeError,
  );
});

test('production search verifier accepts valid output and rejects incomplete artifacts', async (t) => {
  const directory = mkdtempSync(path.join(tmpdir(), 'docs-search-contract-'));
  try {
    mkdirSync(path.join(directory, 'lib'));
    mkdirSync(path.join(directory, 'build'));
    writeFileSync(path.join(directory, 'lib/check-search-index.mjs'),
      readFileSync(new URL('./check-search-index.mjs', import.meta.url)));
    const docs = path.join(directory, 'build/search-index-docs-default-current.json');
    const pages = path.join(directory, 'build/search-index-default.json');
    const terms = ['installation', 'backup', 'dashboard', 'architecture', 'api', 'release'];
    const run = () => spawnSync(process.execPath, [path.join(directory, 'lib/check-search-index.mjs')], {
      encoding: 'utf8',
      env: {PATH: process.env.PATH, LANG: 'C.UTF-8'},
    });
    await t.test('both artifacts are required before content validation', () => {
      let result = run();
      assert.notEqual(result.status, 0);
      assert.match(result.stderr, /Missing production search index: build\/search-index-docs-default-current.json/);
      writeFileSync(docs, 'invalid');
      result = run();
      assert.notEqual(result.status, 0);
      assert.match(result.stderr, /Missing production search index: build\/search-index-default.json/);
    });
    writeFileSync(pages, '{"documents":[{"sectionRoute":"/"}]}');
    for (const missing of terms) {
      await t.test(`missing ${missing} is rejected`, () => {
        writeFileSync(docs, JSON.stringify(terms.filter((term) => term !== missing)));
        const result = run();
        assert.notEqual(result.status, 0);
        assert.match(result.stderr, new RegExp(`missing the term: ${missing}`));
      });
    }
    await t.test('home route and case-insensitive document terms are required', () => {
      writeFileSync(docs, JSON.stringify(terms).toUpperCase());
      writeFileSync(pages, '{"documents":[{"sectionRoute":"/other"}]}');
      assert.match(run().stderr, /does not include the branded homepage/);
      writeFileSync(pages, '{"documents":[{"sectionRoute":"/"}]}');
      const result = run();
      assert.equal(result.status, 0, result.stderr);
      assert.equal(result.stdout, 'Production search indexes contain the homepage and required documentation terms.\n');
    });
  } finally {
    rmSync(directory, {recursive: true, force: true});
  }
});
