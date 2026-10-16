import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { test } from 'node:test';

const docsRequire = createRequire(new URL('../apps/docs/package.json', import.meta.url));
const coreRequire = createRequire(docsRequire.resolve('@docusaurus/core/package.json'));
const mdxRequire = createRequire(coreRequire.resolve('@docusaurus/mdx-loader'));
let packageRoot = dirname(mdxRequire.resolve('image-size'));
while (
  !existsSync(join(packageRoot, 'package.json')) ||
  JSON.parse(readFileSync(join(packageRoot, 'package.json'), 'utf8')).name !== 'image-size'
) {
  packageRoot = dirname(packageRoot);
}

function box(name, payload = Buffer.alloc(0), size = payload.length + 8) {
  const buffer = Buffer.alloc(payload.length + 8);
  buffer.writeUInt32BE(size);
  buffer.write(name, 4);
  payload.copy(buffer, 8);
  return buffer;
}
function icns(size = 8) {
  const entry = Buffer.alloc(8);
  entry.write('ic07');
  entry.writeUInt32BE(size, 4);
  const buffer = Buffer.concat([Buffer.from('icns'), Buffer.alloc(4), entry]);
  buffer.writeUInt32BE(buffer.length, 4);
  return buffer;
}
function heif(size = 20) {
  const dimensions = Buffer.alloc(12);
  dimensions.writeUInt32BE(64, 4);
  dimensions.writeUInt32BE(32, 8);
  return Buffer.concat([
    box('ftyp', Buffer.from('heic\0\0\0\0')),
    box(
      'meta',
      Buffer.concat([Buffer.alloc(4), box('iprp', box('ipco', box('ispe', dimensions, size)))])
    ),
  ]);
}
function jxl(size = 16) {
  return Buffer.concat([
    box('JXL ', Buffer.from([13, 10, 135, 10])),
    box('ftyp', Buffer.from('jxl \0\0\0\0jxl ')),
    box('jxlp', Buffer.from([0, 0, 0, 0, 255, 10, 65, 0]), size),
  ]);
}

// A subprocess timeout also catches synchronous infinite loops that defeat test timeouts.
const exercise = `
  import assert from 'node:assert/strict';
  import { readFileSync } from 'node:fs';
  import { createRequire } from 'node:module';
  const require = createRequire(import.meta.url);
  const load = process.argv[1] === 'cjs' ? async (name) => require(name) : (name) => import(name);
  const { imageSize } = await load('image-size');
  const { imageSizeFromFile } = await load('image-size/fromFile');
  const fixtures = JSON.parse(readFileSync(process.argv[2], 'utf8'));
  for (const { file, expected } of fixtures) {
    const input = readFileSync(file);
    if (expected) {
      for (const actual of [imageSize(input), await imageSizeFromFile(file)]) {
        assert.equal(actual.width, expected.width, file);
        assert.equal(actual.height, expected.height, file);
      }
    } else {
      assert.throws(() => imageSize(input), undefined, file);
      await assert.rejects(() => imageSizeFromFile(file), undefined, file);
    }
  }
  console.log('Checked ' + fixtures.length + ' fixtures via buffer and file APIs');
`;

for (const mode of ['cjs', 'esm']) {
  test(`image-size ${mode}: malformed ICNS/HEIF/JXL terminate; valid dimensions survive`, () => {
    const directory = mkdtempSync(join(tmpdir(), 'panda-images-'));
    try {
      const fixtures = [];
      const add = (name, data, expected) => {
        const file = join(directory, name);
        writeFileSync(file, data);
        fixtures.push({ file, expected });
      };
      add('valid.icns', icns(), { width: 128, height: 128 });
      add('valid.heic', heif(), { width: 64, height: 32 });
      add('valid.jxl', jxl(), { width: 8, height: 8 });
      for (const size of [0, 1, 7]) {
        add(`length-${size}.icns`, icns(size));
        add(`length-${size}.heic`, heif(size));
        add(`length-${size}.jxl`, jxl(size));
      }
      add('truncated.icns', icns().subarray(0, 13));
      add('truncated.heic', heif().subarray(0, 50));
      add('truncated.jxl', jxl().subarray(0, 40));
      const manifest = join(directory, 'fixtures.json');
      writeFileSync(manifest, JSON.stringify(fixtures));
      const result = spawnSync(
        process.execPath,
        ['--input-type=module', '--eval', exercise, mode, manifest],
        {
          cwd: packageRoot,
          encoding: 'utf8',
          timeout: 5000,
        }
      );
      assert.equal(result.error, undefined, `Parser timed out: ${result.error}`);
      assert.equal(result.status, 0, result.stderr);
      assert.match(result.stdout, /Checked 15 fixtures/);
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  });
}
