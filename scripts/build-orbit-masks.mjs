#!/usr/bin/env node
/**
 * Build the body-paint mask pack for the bundled 360° orbit frames.
 *
 * Input:  apps/web/src/features/r-experience/assets/r2-orbit.{bin,json}
 * Output: apps/web/src/features/r-experience/assets/r2-orbit-mask.{bin,json}
 *
 * Each mask marks the painted body in one frame. The selection is deterministic:
 * 1. Keep opaque pixels whose OKLab hue and chroma fall in the band of the
 *    bundled teal paint (glass, tyres and trim are near-neutral and drop out).
 * 2. Clean the selection: close (fill seams and small highlights), open (drop
 *    specks), remove small islands and fill small enclosed holes.
 * 3. Snap the edge to the full-resolution lightness with a guided filter (the
 *    WebP chroma is half resolution and blocky), feather, multiply by alpha.
 * The masks are 8-bit greyscale PNGs, cropped to their bounding box and packed
 * end to end. The index lists [offset, length, x, y, width, height] per frame
 * plus the OKLab body reference (median of the rest frame) that the runtime
 * tint maps onto the owner's paint.
 *
 * Needs ffmpeg on PATH (decodes the WebP frames; VP8 decoding is bit-exact).
 *
 * Usage:
 *   node scripts/build-orbit-masks.mjs                 rebuild the pack
 *   node scripts/build-orbit-masks.mjs --preview DIR   also write overlay PNGs
 *   node scripts/build-orbit-masks.mjs --check DIR     tint frame 56 to the paint
 *       sampled from public/vehicle-images/fallbacks/r2s/side.webp, report ΔE and
 *       write before/after PNGs to DIR (does not rewrite the pack)
 *   ... --check DIR --paint L,a,b   same, with a given OKLab paint
 */
import { execFileSync } from 'node:child_process';
import { mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { crc32, deflateSync } from 'node:zlib';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const ASSETS = path.join(ROOT, 'apps/web/src/features/r-experience/assets');
const WIDTH = 840;
const HEIGHT = 430;
const REST_FRAME = 56;
const MASK_BUDGET = 1.5 * 1024 * 1024;

// Paint band of the bundled teal body (OKLab). Measured on the frames:
// body hue 205°–245°, chroma 0.02–0.05; glass and tyres sit below 0.015 chroma.
const BAND = { hueLow: 200, hueHigh: 250, hueSoft: 6, chromaLow: 0.012, chromaHigh: 0.02, lightLow: 0.17, lightHigh: 0.86 };
const MIN_ISLAND = 120;
const MAX_HOLE = 260;
const GUIDE_RADIUS = 4;
const GUIDE_EPS = 0.0004;

const args = process.argv.slice(2);
const flag = name => { const at = args.indexOf(name); return at >= 0 ? args[at + 1] : undefined; };

// --- colour -----------------------------------------------------------------
const toLinear = new Float64Array(256).map((_, i) => { const c = i / 255; return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4; });
function oklab(r, g, b) {
  const lr = toLinear[r], lg = toLinear[g], lb = toLinear[b];
  const l = Math.cbrt(0.4122214708 * lr + 0.5363325363 * lg + 0.0514459929 * lb);
  const m = Math.cbrt(0.2119034982 * lr + 0.6806995451 * lg + 0.1073969566 * lb);
  const s = Math.cbrt(0.0883024619 * lr + 0.2817188376 * lg + 0.6299787005 * lb);
  return [0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s, 1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s, 0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s];
}
const ramp = (x, a, b) => Math.max(0, Math.min(1, (x - a) / (b - a)));

// --- image io ---------------------------------------------------------------
function decode(bytes, format = 'webp_pipe', pixFmt = 'rgba') {
  return execFileSync('ffmpeg', ['-loglevel', 'error', '-f', format, '-i', 'pipe:0', '-f', 'rawvideo', '-pix_fmt', pixFmt, 'pipe:1'], { input: bytes, maxBuffer: 1 << 28 });
}
function probeSize(bytes, format = 'webp_pipe') {
  const out = execFileSync('ffprobe', ['-v', 'error', '-f', format, '-i', 'pipe:0', '-show_entries', 'stream=width,height', '-of', 'csv=p=0'], { input: bytes }).toString().trim();
  const [width, height] = out.split(',').map(Number);
  return { width, height };
}
function png(width, height, channels, data) {
  const colorType = channels === 1 ? 0 : channels === 3 ? 2 : 6;
  const stride = width * channels;
  const raw = Buffer.alloc((stride + 1) * height);
  for (let y = 0; y < height; y++) {
    // Filter "Up" (2) except the first row: masks are mostly vertical runs.
    raw[y * (stride + 1)] = y === 0 ? 0 : 2;
    for (let x = 0; x < stride; x++) {
      const value = data[y * stride + x];
      raw[y * (stride + 1) + 1 + x] = y === 0 ? value : (value - data[(y - 1) * stride + x]) & 255;
    }
  }
  const chunk = (type, body) => {
    const head = Buffer.alloc(8); head.writeUInt32BE(body.length, 0); head.write(type, 4, 'ascii');
    const crc = Buffer.alloc(4); crc.writeUInt32BE(crc32(Buffer.concat([Buffer.from(type, 'ascii'), body])) >>> 0, 0);
    return Buffer.concat([head, body, crc]);
  };
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0); ihdr.writeUInt32BE(height, 4); ihdr[8] = 8; ihdr[9] = colorType;
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', ihdr), chunk('IDAT', deflateSync(raw, { level: 9 })), chunk('IEND', Buffer.alloc(0))]);
}

// --- morphology on a binary Uint8Array (0/1) ----------------------------------
function dilate(src, r) {
  const tmp = new Uint8Array(src.length), out = new Uint8Array(src.length);
  for (let y = 0; y < HEIGHT; y++) for (let x = 0; x < WIDTH; x++) {
    let v = 0; for (let d = -r; d <= r && !v; d++) { const xx = x + d; if (xx >= 0 && xx < WIDTH) v = src[y * WIDTH + xx]; }
    tmp[y * WIDTH + x] = v;
  }
  for (let y = 0; y < HEIGHT; y++) for (let x = 0; x < WIDTH; x++) {
    let v = 0; for (let d = -r; d <= r && !v; d++) { const yy = y + d; if (yy >= 0 && yy < HEIGHT) v = tmp[yy * WIDTH + x]; }
    out[y * WIDTH + x] = v;
  }
  return out;
}
const invert = src => src.map(v => 1 - v);
const erode = (src, r) => invert(dilate(invert(src), r));
function components(src, value, visit) {
  const seen = new Uint8Array(src.length), stack = new Int32Array(src.length);
  for (let start = 0; start < src.length; start++) {
    if (seen[start] || src[start] !== value) continue;
    let top = 0, size = 0, edge = false; stack[top++] = start; seen[start] = 1;
    const members = [];
    while (top) {
      const at = stack[--top]; members.push(at); size++;
      const x = at % WIDTH, y = (at / WIDTH) | 0;
      if (x === 0 || y === 0 || x === WIDTH - 1 || y === HEIGHT - 1) edge = true;
      for (const next of [x > 0 ? at - 1 : -1, x < WIDTH - 1 ? at + 1 : -1, y > 0 ? at - WIDTH : -1, y < HEIGHT - 1 ? at + WIDTH : -1]) {
        if (next >= 0 && !seen[next] && src[next] === value) { seen[next] = 1; stack[top++] = next; }
      }
    }
    visit(members, size, edge);
  }
}

function boxMean(src, r) {
  // Integral image, clamped window.
  const sum = new Float64Array((WIDTH + 1) * (HEIGHT + 1));
  for (let y = 0; y < HEIGHT; y++) {
    let row = 0;
    for (let x = 0; x < WIDTH; x++) { row += src[y * WIDTH + x]; sum[(y + 1) * (WIDTH + 1) + x + 1] = sum[y * (WIDTH + 1) + x + 1] + row; }
  }
  const out = new Float32Array(src.length);
  for (let y = 0; y < HEIGHT; y++) for (let x = 0; x < WIDTH; x++) {
    const x0 = Math.max(0, x - r), x1 = Math.min(WIDTH, x + r + 1), y0 = Math.max(0, y - r), y1 = Math.min(HEIGHT, y + r + 1);
    const total = sum[y1 * (WIDTH + 1) + x1] - sum[y0 * (WIDTH + 1) + x1] - sum[y1 * (WIDTH + 1) + x0] + sum[y0 * (WIDTH + 1) + x0];
    out[y * WIDTH + x] = total / ((x1 - x0) * (y1 - y0));
  }
  return out;
}
function guidedFilter(guide, input, r, eps) {
  const product = (p, q) => p.map((v, i) => v * q[i]);
  const meanI = boxMean(guide, r), meanP = boxMean(input, r);
  const corrII = boxMean(product(guide, guide), r), corrIP = boxMean(product(guide, input), r);
  const a = new Float32Array(guide.length), b = new Float32Array(guide.length);
  for (let i = 0; i < guide.length; i++) {
    a[i] = (corrIP[i] - meanI[i] * meanP[i]) / (corrII[i] - meanI[i] * meanI[i] + eps);
    b[i] = meanP[i] - a[i] * meanI[i];
  }
  const meanA = boxMean(a, r), meanB = boxMean(b, r);
  return guide.map((v, i) => meanA[i] * v + meanB[i]);
}

// --- mask -------------------------------------------------------------------
function buildMask(rgba) {
  const n = WIDTH * HEIGHT;
  const score = new Float32Array(n);
  const seed = new Uint8Array(n);
  const opaque = new Uint8Array(n);
  // Clearly coloured pixels of another hue (amber markers, red lamps) never join the mask.
  const foreign = new Uint8Array(n);
  for (let i = 0; i < n; i++) {
    const p = i * 4;
    if (rgba[p + 3] < 200) continue;
    opaque[i] = 1;
    const [L, a, b] = oklab(rgba[p], rgba[p + 1], rgba[p + 2]);
    const C = Math.hypot(a, b);
    const h = (Math.atan2(b, a) * 180 / Math.PI + 360) % 360;
    const hue = Math.min(ramp(h, BAND.hueLow - BAND.hueSoft, BAND.hueLow), 1 - ramp(h, BAND.hueHigh, BAND.hueHigh + BAND.hueSoft));
    const light = Math.min(ramp(L, BAND.lightLow - 0.04, BAND.lightLow), 1 - ramp(L, BAND.lightHigh, BAND.lightHigh + 0.04));
    score[i] = hue * light * ramp(C, BAND.chromaLow, BAND.chromaHigh);
    foreign[i] = C >= 0.03 && hue === 0 ? 1 : 0;
    seed[i] = score[i] >= 0.5 ? 1 : 0;
  }
  // Close seams and small highlights, then open to drop specks.
  let mask = erode(dilate(seed, 2), 2);
  mask = dilate(erode(mask, 1), 1);
  for (let i = 0; i < n; i++) mask[i] &= opaque[i] & (1 - foreign[i]);
  components(mask, 1, (members, size) => { if (size < MIN_ISLAND) for (const at of members) mask[at] = 0; });
  components(mask, 0, (members, size, edge) => {
    if (edge || size > MAX_HOLE) return;
    for (const at of members) if (opaque[at] && !foreign[at]) mask[at] = 1;
  });
  // WebP stores chroma at half resolution with block artefacts, so the edge of the
  // chroma selection is blocky. A guided filter (He et al.) with the
  // full-resolution OKLab lightness as guide snaps it to the real panel edges.
  const guide = new Float32Array(n);
  for (let i = 0; i < n; i++) guide[i] = opaque[i] ? oklab(rgba[i * 4], rgba[i * 4 + 1], rgba[i * 4 + 2])[0] : 1;
  const refined = guidedFilter(guide, Float32Array.from(mask), GUIDE_RADIUS, GUIDE_EPS);
  const weight = new Float32Array(n);
  for (let i = 0; i < n; i++) weight[i] = opaque[i] ? ramp(refined[i], 0.2, 0.8) : 0;
  // One-pixel feather (3×3 mean) times alpha.
  const soft = new Uint8Array(n);
  for (let y = 0; y < HEIGHT; y++) for (let x = 0; x < WIDTH; x++) {
    let sum = 0;
    for (let dy = -1; dy <= 1; dy++) for (let dx = -1; dx <= 1; dx++) {
      const xx = Math.min(WIDTH - 1, Math.max(0, x + dx)), yy = Math.min(HEIGHT - 1, Math.max(0, y + dy));
      sum += weight[yy * WIDTH + xx];
    }
    const i = y * WIDTH + x;
    soft[i] = Math.round((sum / 9) * rgba[i * 4 + 3]);
  }
  return soft;
}

function crop(mask) {
  let x0 = WIDTH, y0 = HEIGHT, x1 = -1, y1 = -1;
  for (let y = 0; y < HEIGHT; y++) for (let x = 0; x < WIDTH; x++) {
    if (!mask[y * WIDTH + x]) continue;
    if (x < x0) x0 = x; if (x > x1) x1 = x; if (y < y0) y0 = y; if (y > y1) y1 = y;
  }
  if (x1 < 0) return { x: 0, y: 0, width: 1, height: 1, data: new Uint8Array(1) };
  const width = x1 - x0 + 1, height = y1 - y0 + 1, data = new Uint8Array(width * height);
  for (let y = 0; y < height; y++) data.set(mask.subarray((y + y0) * WIDTH + x0, (y + y0) * WIDTH + x0 + width), y * width);
  return { x: x0, y: y0, width, height, data };
}

function bodyReference(rgba, mask) {
  const L = [], C = [], sinH = [], cosH = [];
  for (let i = 0; i < mask.length; i++) {
    if (mask[i] < 128) continue;
    const [l, a, b] = oklab(rgba[i * 4], rgba[i * 4 + 1], rgba[i * 4 + 2]);
    L.push(l); C.push(Math.hypot(a, b)); sinH.push(b); cosH.push(a);
  }
  const median = values => { const sorted = Float64Array.from(values).sort(); return sorted[sorted.length >> 1]; };
  const hue = Math.atan2(sinH.reduce((s, v) => s + v, 0), cosH.reduce((s, v) => s + v, 0)) * 180 / Math.PI;
  const round = v => Math.round(v * 10000) / 10000;
  return { L: round(median(L)), C: round(median(C)), h: round((hue + 360) % 360) };
}

const frameIndex = JSON.parse(readFileSync(path.join(ASSETS, 'r2-orbit.json'), 'utf8'));
const framePack = readFileSync(path.join(ASSETS, 'r2-orbit.bin'));
const frameBytes = index => framePack.subarray(frameIndex[index][0], frameIndex[index][0] + frameIndex[index][1]);

async function check(outDir) {
  // Bundle the runtime module so the check uses the exact tint and sampling code.
  // esbuild is a dependency of the web app's Vite toolchain.
  const { build } = await import(pathToFileURL(createRequire(path.join(ROOT, 'apps/web/package.json')).resolve('esbuild')).href);
  const bundled = path.join(outDir, '.orbitFrames.check.mjs');
  await build({
    entryPoints: [path.join(ROOT, 'apps/web/src/features/r-experience/orbitFrames.ts')], bundle: true, format: 'esm', platform: 'node', outfile: bundled, logLevel: 'error',
    plugins: [{ name: 'asset-url', setup(b) { b.onResolve({ filter: /\?url$/ }, a => ({ path: a.path, namespace: 'asset-url' })); b.onLoad({ filter: /.*/, namespace: 'asset-url' }, a => ({ contents: `export default ${JSON.stringify(a.path)};`, loader: 'js' })); } }],
  });
  const orbit = await import(pathToFileURL(bundled).href);
  rmSync(bundled);
  const maskIndex = JSON.parse(readFileSync(path.join(ASSETS, 'r2-orbit-mask.json'), 'utf8'));
  const maskPack = readFileSync(path.join(ASSETS, 'r2-orbit-mask.bin'));
  orbit.validateMaskIndex(maskIndex, maskPack.byteLength);

  const sideBytes = readFileSync(path.join(ROOT, 'apps/web/public/vehicle-images/fallbacks/r2s/side.webp'));
  const side = probeSize(sideBytes);
  const override = flag('--paint')?.split(',').map(Number);
  const paint = override ?? orbit.sampleArtworkPaint(new Uint8ClampedArray(decode(sideBytes)), side.width, side.height);
  const suffix = override ? `-${override.map(v => v.toFixed(2)).join('_')}` : '';
  if (!paint) throw new Error('No paint could be sampled from the side artwork.');

  const before = new Uint8ClampedArray(decode(frameBytes(REST_FRAME)));
  const [offset, length, x, y, w, h] = maskIndex.frames[REST_FRAME];
  const local = decode(maskPack.subarray(offset, offset + length), 'png_pipe', 'gray');
  const mask = new Uint8Array(WIDTH * HEIGHT);
  for (let row = 0; row < h; row++) mask.set(local.subarray(row * w, row * w + w), (row + y) * WIDTH + x);
  const after = before.slice();
  orbit.tintPixels(after, mask, paint, maskIndex.reference);

  const body = [];
  for (let i = 0; i < mask.length; i++) if (mask[i] >= 250) body.push(orbit.srgbToOklab(after[i * 4], after[i * 4 + 1], after[i * 4 + 2]));
  const median = k => { const s = body.map(v => v[k]).sort((p, q) => p - q); return s[s.length >> 1]; };
  const tinted = [median(0), median(1), median(2)];
  const deltaE = Math.hypot(tinted[0] - paint[0], tinted[1] - paint[1], tinted[2] - paint[2]) * 100;
  let changedOutside = 0;
  for (let i = 0; i < mask.length; i++) if (!mask[i] && (after[i * 4] !== before[i * 4] || after[i * 4 + 1] !== before[i * 4 + 1] || after[i * 4 + 2] !== before[i * 4 + 2])) changedOutside++;

  const rgbaPng = data => png(WIDTH, HEIGHT, 4, data);
  writeFileSync(path.join(outDir, 'frame56-before.png'), rgbaPng(before));
  writeFileSync(path.join(outDir, `frame56-after${suffix}.png`), rgbaPng(after));
  writeFileSync(path.join(outDir, 'frame56-mask.png'), png(WIDTH, HEIGHT, 1, mask));
  const fmt = v => v.map(n => n.toFixed(4)).join(', ');
  console.log(`paint ${override ? 'given' : 'sampled from r2s/side.webp'} (OKLab): ${fmt(paint)}  sRGB ${orbit.oklabToSrgb(...paint).join(',')}`);
  console.log(`tinted body median, frame ${REST_FRAME} (OKLab): ${fmt(tinted)}  over ${body.length} body pixels`);
  console.log(`ΔE (OKLab×100) = ${deltaE.toFixed(2)}  (target <= 10)`);
  console.log(`pixels changed outside the mask: ${changedOutside}`);
  return deltaE <= 10 && changedOutside === 0;
}

if (args.includes('--check')) {
  const outDir = path.resolve(flag('--check'));
  mkdirSync(outDir, { recursive: true });
  const ok = await check(outDir);
  process.exit(ok ? 0 : 1);
}

const preview = flag('--preview');
if (preview) mkdirSync(preview, { recursive: true });
const chunks = [];
const entries = [];
let offset = 0;
let reference;
for (let index = 0; index < frameIndex.length; index++) {
  const rgba = decode(frameBytes(index));
  if (rgba.length !== WIDTH * HEIGHT * 4) throw new Error(`Frame ${index} is not ${WIDTH}×${HEIGHT}.`);
  const mask = buildMask(rgba);
  if (index === REST_FRAME) reference = bodyReference(rgba, mask);
  const box = crop(mask);
  const bytes = png(box.width, box.height, 1, box.data);
  entries.push([offset, bytes.length, box.x, box.y, box.width, box.height]);
  chunks.push(bytes);
  offset += bytes.length;
  if (preview) {
    const overlay = Buffer.from(rgba);
    for (let i = 0; i < mask.length; i++) {
      const m = mask[i] / 255;
      overlay[i * 4] = Math.round(overlay[i * 4] * (1 - m) + 255 * m);
      overlay[i * 4 + 1] = Math.round(overlay[i * 4 + 1] * (1 - m * 0.7));
      overlay[i * 4 + 3] = Math.max(overlay[i * 4 + 3], 255 * (mask[i] ? 1 : 0));
    }
    writeFileSync(path.join(preview, `mask-${String(index).padStart(2, '0')}.png`), png(WIDTH, HEIGHT, 4, overlay));
  }
}
if (offset > MASK_BUDGET) throw new Error(`Mask pack is ${offset} bytes; the budget is ${MASK_BUDGET}.`);
writeFileSync(path.join(ASSETS, 'r2-orbit-mask.bin'), Buffer.concat(chunks));
writeFileSync(path.join(ASSETS, 'r2-orbit-mask.json'), `${JSON.stringify({ version: 1, width: WIDTH, height: HEIGHT, format: 'png-gray8', reference, frames: entries })}\n`);
console.log(`Wrote ${entries.length} masks, ${offset} bytes. Rest-frame body reference (OKLab L, C, h°): ${JSON.stringify(reference)}`);
