import frameIndex from './assets/r2-orbit.json';
import packUrl from './assets/r2-orbit.bin?url';
import maskIndexJson from './assets/r2-orbit-mask.json';
import maskUrl from './assets/r2-orbit-mask.bin?url';

export const ORBIT_FRAMES = 72;
export const DEFAULT_FRAME = 56;
export const FRONT_FRAME = 65;
export const BITMAP_LIMIT = 6;
export const FRAME_WIDTH = 840;
export const FRAME_HEIGHT = 430;
let pack: Promise<ArrayBuffer> | undefined;
let maskPack: Promise<ArrayBuffer> | undefined;

/** A colour in OKLab: [L, a, b]. */
export type Oklab = readonly [number, number, number];
/** The bundled body paint in the rest frame (OKLab lightness, chroma, hue in degrees). */
export interface BodyReference { L: number; C: number; h: number }
/** [offset, length, x, y, width, height] of one cropped 8-bit mask PNG in the pack. */
export type MaskEntry = [number, number, number, number, number, number];
export interface MaskIndex {
  version: 1;
  width: number;
  height: number;
  format: 'png-gray8';
  reference: BodyReference;
  frames: MaskEntry[];
}

export function wrapFrame(frame: number) {
  return ((Math.round(frame) % ORBIT_FRAMES) + ORBIT_FRAMES) % ORBIT_FRAMES;
}

// --- owner-image turn ---------------------------------------------------------
// The owner's own Rivian images form a short angle set (front, front three-quarter,
// side, rear three-quarter, rear). These helpers step through that set.

/** An angle index in a set of `count` images, rounded and wrapped around. */
export function wrapAngleIndex(index: number, count: number) {
  if (count < 1) return 0;
  return ((Math.round(index) % count) + count) % count;
}

/** The angle the car rests on: side, else the nearest three-quarter, else the first. */
export function restAngleIndex(angles: ReadonlyArray<{ angle: string }>) {
  for (const angle of ['side', 'front-three-quarter', 'rear-three-quarter']) {
    const index = angles.findIndex(item => item.angle === angle);
    if (index >= 0) return index;
  }
  return 0;
}

/** Drag distance in CSS px for one angle step. */
export function angleStepWidth(width: number) {
  return Math.max(48, Math.min(140, width / 4));
}

/** A released drag carries on for this long at its release velocity before it rounds. */
export const ANGLE_INERTIA_MS = 180;

/**
 * The angle a released drag settles on. `position` is the fractional angle index under
 * the pointer and `velocity` is in angle steps per ms. Inertia adds at most one step;
 * the result is rounded to the nearest angle and wrapped around.
 */
export function settleAngle(position: number, velocity: number, count: number) {
  const carry = Math.max(-1, Math.min(1, velocity * ANGLE_INERTIA_MS));
  return wrapAngleIndex(position + carry, count);
}

/** The next angle index for a key press, or null when the key does not turn the car. */
export function angleForKey(key: string, index: number, count: number, rest: number): number | null {
  if (key === 'ArrowRight' || key === 'ArrowUp') return wrapAngleIndex(index + 1, count);
  if (key === 'ArrowLeft' || key === 'ArrowDown') return wrapAngleIndex(index - 1, count);
  if (key === 'Home') return rest;
  if (key === 'End') return Math.max(0, count - 1);
  return null;
}

/** Throws when the mask index does not describe one in-bounds mask per frame, packed end to end. */
export function validateMaskIndex(index: MaskIndex, byteLength: number): MaskIndex {
  const fail = (reason: string) => { throw new Error(`Invalid vehicle paint masks: ${reason}.`); };
  if (index.version !== 1 || index.format !== 'png-gray8') fail('unknown format');
  if (index.width !== FRAME_WIDTH || index.height !== FRAME_HEIGHT) fail('frame size differs');
  if (index.frames.length !== ORBIT_FRAMES) fail('frame count differs');
  const { L, C, h } = index.reference;
  if (![L, C, h].every(Number.isFinite) || L <= 0 || L >= 1 || C <= 0) fail('no body reference');
  let next = 0;
  for (const [offset, length, x, y, width, height] of index.frames) {
    if (offset !== next || length <= 0) fail('entries are not contiguous');
    if (x < 0 || y < 0 || width < 1 || height < 1 || x + width > FRAME_WIDTH || y + height > FRAME_HEIGHT) fail('mask outside the frame');
    next = offset + length;
  }
  if (next !== byteLength) fail('pack size differs');
  return index;
}

const maskIndex = maskIndexJson as MaskIndex;

// --- colour -----------------------------------------------------------------

const toLinear = Float32Array.from({ length: 256 }, (_, i) => {
  const c = i / 255;
  return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
});
function toSrgb(linear: number) {
  const c = linear <= 0.0031308 ? linear * 12.92 : 1.055 * linear ** (1 / 2.4) - 0.055;
  return c * 255;
}

export function srgbToOklab(r: number, g: number, b: number): [number, number, number] {
  const lr = toLinear[r]!, lg = toLinear[g]!, lb = toLinear[b]!;
  const l = Math.cbrt(0.4122214708 * lr + 0.5363325363 * lg + 0.0514459929 * lb);
  const m = Math.cbrt(0.2119034982 * lr + 0.6806995451 * lg + 0.1073969566 * lb);
  const s = Math.cbrt(0.0883024619 * lr + 0.2817188376 * lg + 0.6299787005 * lb);
  return [
    0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
    1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
    0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
  ];
}

function oklabToLinear(L: number, a: number, b: number): [number, number, number] {
  const l = (L + 0.3963377774 * a + 0.2158037573 * b) ** 3;
  const m = (L - 0.1055613458 * a - 0.0638541728 * b) ** 3;
  const s = (L - 0.0894841775 * a - 1.291485548 * b) ** 3;
  return [
    4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
    -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
    -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
  ];
}
const inGamut = (rgb: [number, number, number]) => rgb.every(c => c >= -1e-4 && c <= 1 + 1e-4);

/** OKLab to 8-bit sRGB. Out-of-gamut colours keep lightness and hue and lose chroma. */
export function oklabToSrgb(L: number, a: number, b: number): [number, number, number] {
  let linear = oklabToLinear(L, a, b);
  if (!inGamut(linear)) {
    let low = 0, high = 1;
    for (let step = 0; step < 12; step++) {
      const mid = (low + high) / 2;
      if (inGamut(oklabToLinear(L, a * mid, b * mid))) low = mid; else high = mid;
    }
    linear = oklabToLinear(L, a * low, b * low);
  }
  return linear.map(c => Math.round(Math.min(255, Math.max(0, toSrgb(Math.min(1, Math.max(0, c))))))) as [number, number, number];
}

/**
 * Lightness tone curve that maps the bundled body reference onto the paint and
 * keeps the shading around it. The side that has room keeps its contrast; the
 * other side compresses so the result stays inside 0..1 without clipping.
 */
export function mapLightness(L: number, reference: number, paint: number) {
  if (L <= reference) return paint - (reference - L) * (paint >= reference ? 1 : paint / reference);
  return paint + (L - reference) * (paint <= reference ? 1 : (1 - paint) / (1 - reference));
}

/**
 * How much of the bundled paint's own chroma and hue variation carries into the
 * tint. The WebP frames store chroma at half resolution with block artefacts; at
 * full strength a saturated owner paint would magnify them into visible blotches.
 */
export const CHROMA_DETAIL = 0.35;

/**
 * Tints the masked body pixels of one RGBA frame toward `paint`, in place.
 * Lightness follows {@link mapLightness}, so the shading keeps its structure.
 * Chroma and hue take the paint's values plus a damped share of the pixel's
 * deviation from the bundled paint ({@link CHROMA_DETAIL}).
 * `mask` holds one 0..255 coverage byte per pixel; 0 leaves the pixel untouched.
 */
export function tintPixels(rgba: Uint8ClampedArray, mask: ArrayLike<number>, paint: Oklab, reference: BodyReference) {
  const [pL, pa, pb] = paint;
  const paintChroma = Math.hypot(pa, pb);
  const paintHue = Math.atan2(pb, pa);
  const referenceHue = reference.h * Math.PI / 180;
  for (let i = 0; i < mask.length; i++) {
    const coverage = mask[i]!;
    if (!coverage) continue;
    const p = i * 4;
    const [L, a, b] = srgbToOklab(rgba[p]!, rgba[p + 1]!, rgba[p + 2]!);
    const chroma = paintChroma * Math.max(0, 1 + (Math.hypot(a, b) / reference.C - 1) * CHROMA_DETAIL);
    const turn = Math.atan2(Math.sin(Math.atan2(b, a) - referenceHue), Math.cos(Math.atan2(b, a) - referenceHue));
    const hue = paintHue + turn * CHROMA_DETAIL;
    const tinted = oklabToSrgb(mapLightness(L, reference.L, pL), chroma * Math.cos(hue), chroma * Math.sin(hue));
    const weight = coverage / 255;
    rgba[p] = rgba[p]! + (tinted[0] - rgba[p]!) * weight;
    rgba[p + 1] = rgba[p + 1]! + (tinted[1] - rgba[p + 1]!) * weight;
    rgba[p + 2] = rgba[p + 2]! + (tinted[2] - rgba[p + 2]!) * weight;
  }
}

/**
 * Dominant body colour of a side-profile vehicle artwork (RGBA, transparent
 * background). Looks only at opaque pixels in the door band of the silhouette,
 * drops near-black glass and tyres and the brightest specular highlights, then
 * takes the median of the most populated OKLab cluster. Null when too little
 * paint is visible.
 */
export function sampleArtworkPaint(rgba: Uint8ClampedArray, width: number, height: number): Oklab | null {
  let top = height, bottom = -1, left = width, right = -1;
  for (let y = 0; y < height; y++) for (let x = 0; x < width; x++) {
    if (rgba[(y * width + x) * 4 + 3]! < 250) continue;
    if (y < top) top = y; if (y > bottom) bottom = y; if (x < left) left = x; if (x > right) right = x;
  }
  if (bottom < 0) return null;
  const spanY = bottom - top + 1, spanX = right - left + 1;
  const pixels: Array<[number, number, number]> = [];
  for (let y = Math.round(top + spanY * 0.38); y <= top + spanY * 0.68; y++) {
    for (let x = Math.round(left + spanX * 0.1); x <= left + spanX * 0.9; x++) {
      const p = (y * width + x) * 4;
      if (rgba[p + 3]! < 250) continue;
      const colour = srgbToOklab(rgba[p]!, rgba[p + 1]!, rgba[p + 2]!);
      if (colour[0] >= 0.15) pixels.push(colour);
    }
  }
  if (pixels.length < 64) return null;
  const lightness = pixels.map(colour => colour[0]).sort((p, q) => p - q);
  const specular = lightness[Math.floor(lightness.length * 0.95)]!;
  const bins = new Map<string, number>();
  const key = ([L, a, b]: readonly number[]) => `${Math.floor(L! / 0.04)},${Math.floor(a! / 0.015)},${Math.floor(b! / 0.015)}`;
  let body = pixels.filter(colour => colour[0] <= specular);
  if (body.length < 32) body = pixels;
  for (const colour of body) bins.set(key(colour), (bins.get(key(colour)) ?? 0) + 1);
  const [best] = [...bins].reduce((most, entry) => (entry[1] > most[1] ? entry : most));
  const [bl, ba, bb] = best.split(',').map(Number) as [number, number, number];
  const centre = [(bl + 0.5) * 0.04, (ba + 0.5) * 0.015, (bb + 0.5) * 0.015];
  const cluster = body.filter(([L, a, b]) => Math.abs(L - centre[0]!) <= 0.06 && Math.hypot(a - centre[1]!, b - centre[2]!) <= 0.03);
  const median = (axis: 0 | 1 | 2) => { const values = cluster.map(colour => colour[axis]).sort((p, q) => p - q); return values[values.length >> 1]!; };
  return [median(0), median(1), median(2)];
}

// --- loading ----------------------------------------------------------------

function makeCanvas(width: number, height: number) {
  if (typeof OffscreenCanvas !== 'undefined') return new OffscreenCanvas(width, height);
  const canvas = document.createElement('canvas');
  canvas.width = width;
  canvas.height = height;
  return canvas;
}

/** Owner paint from a side-profile artwork URL (same origin or blob). Null when it cannot be read. */
export async function loadArtworkPaint(source: string): Promise<Oklab | null> {
  try {
    const image = new Image();
    image.decoding = 'async';
    image.src = source;
    await image.decode();
    const scale = Math.min(1, 360 / Math.max(1, image.naturalWidth));
    const width = Math.max(1, Math.round(image.naturalWidth * scale));
    const height = Math.max(1, Math.round(image.naturalHeight * scale));
    const context = makeCanvas(width, height).getContext('2d', { willReadFrequently: true }) as CanvasRenderingContext2D | null;
    if (!context) return null;
    context.drawImage(image, 0, 0, width, height);
    return sampleArtworkPaint(context.getImageData(0, 0, width, height).data, width, height);
  } catch {
    return null;
  }
}

async function readPack() {
  pack ??= fetch(packUrl, { credentials: 'same-origin' }).then(async response => {
    if (!response.ok) throw new Error('Vehicle rotation could not be loaded.');
    const bytes = await response.arrayBuffer();
    const last = frameIndex.at(-1)!;
    if (bytes.byteLength !== last[0]! + last[1]!) throw new Error('Incomplete vehicle rotation.');
    return bytes;
  }).catch(error => {
    pack = undefined;
    throw error;
  });
  return pack;
}

async function readMasks() {
  maskPack ??= fetch(maskUrl, { credentials: 'same-origin' }).then(async response => {
    if (!response.ok) throw new Error('Vehicle paint masks could not be loaded.');
    const bytes = await response.arrayBuffer();
    validateMaskIndex(maskIndex, bytes.byteLength);
    return bytes;
  }).catch(error => {
    maskPack = undefined;
    throw error;
  });
  return maskPack;
}

async function tintFrame(image: ImageBitmap, masks: ArrayBuffer, index: number, paint: Oklab) {
  const [offset, length, x, y, width, height] = maskIndex.frames[index]!;
  const maskImage = await createImageBitmap(new Blob([masks.slice(offset, offset + length)], { type: 'image/png' }),
    { colorSpaceConversion: 'none', premultiplyAlpha: 'none' });
  const canvas = makeCanvas(FRAME_WIDTH, FRAME_HEIGHT);
  const context = canvas.getContext('2d', { willReadFrequently: true }) as CanvasRenderingContext2D | null;
  if (!context) { maskImage.close(); throw new Error('No canvas.'); }
  context.drawImage(maskImage, 0, 0);
  maskImage.close();
  const local = context.getImageData(0, 0, width, height).data;
  const mask = new Uint8Array(FRAME_WIDTH * FRAME_HEIGHT);
  for (let row = 0; row < height; row++) {
    for (let column = 0; column < width; column++) mask[(row + y) * FRAME_WIDTH + x + column] = local[(row * width + column) * 4]!;
  }
  context.clearRect(0, 0, FRAME_WIDTH, FRAME_HEIGHT);
  context.drawImage(image, 0, 0, FRAME_WIDTH, FRAME_HEIGHT);
  const frame = context.getImageData(0, 0, FRAME_WIDTH, FRAME_HEIGHT);
  tintPixels(frame.data, mask, paint, maskIndex.reference);
  context.putImageData(frame, 0, 0);
  return createImageBitmap(canvas);
}

export function createOrbitFrames() {
  const images = new Map<number, ImageBitmap>();
  const pending = new Map<number, Promise<ImageBitmap>>();
  let disposed = false;
  let paint: Oklab | null = null;
  let generation = 0;
  const clear = () => { images.forEach(image => image.close()); images.clear(); };
  async function get(frame: number): Promise<ImageBitmap> {
    const index = wrapFrame(frame);
    const cached = images.get(index);
    if (cached) {
      images.delete(index);
      images.set(index, cached);
      return cached;
    }
    const inflight = pending.get(index);
    if (inflight) return inflight;
    const started = generation;
    const tint = paint;
    const work = (async () => {
      // Paint masks are optional: without them the frames show their own paint.
      const [bytes, masks] = await Promise.all([readPack(), tint ? readMasks().catch(() => null) : null]);
      if (disposed) throw new Error('Rotation closed.');
      const [offset, length] = frameIndex[index]!;
      let image = await createImageBitmap(new Blob([bytes.slice(offset, offset! + length!)], { type: 'image/webp' }));
      if (tint && masks && !disposed) {
        try {
          const tinted = await tintFrame(image, masks, index, tint);
          image.close();
          image = tinted;
        } catch { /* Show the frame untinted. */ }
      }
      if (disposed) { image.close(); throw new Error('Rotation closed.'); }
      if (started !== generation) { image.close(); return get(index); }
      images.set(index, image);
      while (images.size > BITMAP_LIMIT) {
        const oldest = images.keys().next().value!;
        images.get(oldest)!.close();
        images.delete(oldest);
      }
      return image;
    })().finally(() => { if (pending.get(index) === work) pending.delete(index); });
    pending.set(index, work);
    return work;
  }
  return {
    get,
    /** Owner paint for the body, or null for the frames' own paint. Drops cached frames when it changes. */
    setPaint(next: Oklab | null) {
      if (next === paint || (next && paint && next.every((value, axis) => value === paint![axis]))) return;
      paint = next;
      generation++;
      pending.clear();
      clear();
    },
    close() {
      disposed = true;
      clear();
    },
  };
}
