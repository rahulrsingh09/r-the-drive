import { readFileSync, statSync } from 'node:fs';
import { resolve } from 'node:path';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { MaskIndex, Oklab } from './orbitFrames';

afterEach(() => { vi.unstubAllGlobals(); vi.resetModules(); });

const asset = (name: string) => resolve(process.cwd(), 'src/features/r-experience/assets', name);
const FRAME_BYTES = 3460102;
const MASK_BYTES = statSync(asset('r2-orbit-mask.bin')).size;

const sourceOf = (image: ImageBitmap) => (image as unknown as { source: unknown }).source;

function stubPacks({ masks = true } = {}) {
  const fetch = vi.fn(async (url: string) => {
    if (String(url).includes('mask')) {
      return masks ? { ok: true, arrayBuffer: async () => new ArrayBuffer(MASK_BYTES) } : { ok: false, arrayBuffer: async () => new ArrayBuffer(0) };
    }
    return { ok: true, arrayBuffer: async () => new ArrayBuffer(FRAME_BYTES) };
  });
  const images: Array<{ close: ReturnType<typeof vi.fn>; source: unknown }> = [];
  vi.stubGlobal('fetch', fetch);
  vi.stubGlobal('createImageBitmap', vi.fn(async (source: unknown) => {
    const image = { close: vi.fn(), source }; images.push(image); return image;
  }));
  return { fetch, images };
}

it('fetches one packed asset on demand and bounds decoded image retention', async () => {
  const { fetch, images } = stubPacks();
  const { createOrbitFrames, BITMAP_LIMIT } = await import('./orbitFrames');
  const frames = createOrbitFrames();
  expect(fetch).not.toHaveBeenCalled();
  await frames.get(0);
  await frames.get(0);
  expect(images).toHaveLength(1);
  for (let i = 1; i < 12; i++) await frames.get(i);
  expect(images.filter(image => image.close.mock.calls.length === 0)).toHaveLength(BITMAP_LIMIT);
  expect(fetch).toHaveBeenCalledOnce();
  frames.close();
  expect(images.every(image => image.close.mock.calls.length === 1)).toBe(true);
  const next = createOrbitFrames();
  await next.get(56);
  expect(fetch).toHaveBeenCalledOnce();
  next.close();
});

it('wraps negative and repeated rotations without going outside the image set', async () => {
  const { wrapFrame } = await import('./orbitFrames');
  expect(wrapFrame(-1)).toBe(71);
  expect(wrapFrame(72 * 20 + 56)).toBe(56);
  expect(wrapFrame(-72 * 20)).toBe(0);
});

describe('paint mask pack', () => {
  const index = JSON.parse(readFileSync(asset('r2-orbit-mask.json'), 'utf8')) as MaskIndex;

  it('holds one in-bounds PNG mask per frame, packed end to end inside the size budget', async () => {
    const { validateMaskIndex, ORBIT_FRAMES } = await import('./orbitFrames');
    expect(() => validateMaskIndex(index, MASK_BYTES)).not.toThrow();
    expect(index.frames).toHaveLength(ORBIT_FRAMES);
    expect(MASK_BYTES).toBeLessThanOrEqual(1.5 * 1024 * 1024);
    const pack = readFileSync(asset('r2-orbit-mask.bin'));
    for (const [offset] of index.frames) {
      expect([...pack.subarray(offset, offset + 8)]).toEqual([137, 80, 78, 71, 13, 10, 26, 10]);
    }
    // The bundled paint is the teal body: blue-green hue, low chroma, mid-dark.
    expect(index.reference.h).toBeGreaterThan(200);
    expect(index.reference.h).toBeLessThan(250);
  });

  it('rejects an index that does not match its pack', async () => {
    const { validateMaskIndex } = await import('./orbitFrames');
    expect(() => validateMaskIndex(index, MASK_BYTES + 1)).toThrow(/pack size/);
    const gap = structuredClone(index);
    gap.frames[3]![0] += 1;
    expect(() => validateMaskIndex(gap, MASK_BYTES)).toThrow(/contiguous/);
    const outside = structuredClone(index);
    outside.frames[0]![4] = 900;
    expect(() => validateMaskIndex(outside, MASK_BYTES)).toThrow(/outside/);
    expect(() => validateMaskIndex({ ...index, frames: index.frames.slice(1) }, MASK_BYTES)).toThrow(/count/);
  });
});

describe('tint', () => {
  const reference = { L: 0.316, C: 0.034, h: 230.5 };

  it('maps the body reference onto the paint and keeps lightness inside 0..1', async () => {
    const { mapLightness } = await import('./orbitFrames');
    for (const paint of [0.05, 0.25, 0.316, 0.6, 0.97]) {
      expect(mapLightness(reference.L, reference.L, paint)).toBeCloseTo(paint, 10);
      expect(mapLightness(0, reference.L, paint)).toBeGreaterThanOrEqual(0);
      expect(mapLightness(1, reference.L, paint)).toBeLessThanOrEqual(1 + 1e-12);
      expect(mapLightness(0.2, reference.L, paint)).toBeLessThan(mapLightness(0.4, reference.L, paint));
    }
  });

  it('keeps the luminance shading of the body within tolerance', async () => {
    const { tintPixels, srgbToOklab, oklabToSrgb, mapLightness } = await import('./orbitFrames');
    // Shaded teal body, from a deep shadow to a lit panel.
    const body = [0.18, 0.24, 0.3, 0.316, 0.36, 0.44, 0.55].map(L => oklabToSrgb(L, -0.022, -0.026));
    const paints: Oklab[] = [[0.95, -0.002, 0.004], [0.52, 0.17, 0.08], [0.25, -0.005, -0.03], [0.366, -0.002, -0.008]];
    for (const paint of paints) {
      const rgba = new Uint8ClampedArray(body.flatMap(([r, g, b]) => [r, g, b, 255]));
      tintPixels(rgba, new Uint8Array(body.length).fill(255), paint, reference);
      const before = body.map(([r, g, b]) => srgbToOklab(r, g, b)[0]);
      const after = body.map((_, i) => srgbToOklab(rgba[i * 4]!, rgba[i * 4 + 1]!, rgba[i * 4 + 2]!)[0]);
      after.forEach((L, i) => expect(Math.abs(L - mapLightness(before[i]!, reference.L, paint[0]))).toBeLessThan(0.01));
      for (let i = 1; i < after.length; i++) expect(after[i]!).toBeGreaterThan(after[i - 1]!);
      // The reference shade lands on the paint (OKLab ΔE×100 well under 10).
      const [L, a, b] = srgbToOklab(rgba[12]!, rgba[13]!, rgba[14]!);
      expect(Math.hypot(L - paint[0], a - paint[1], b - paint[2]) * 100).toBeLessThan(3);
    }
  });

  it('leaves unmasked pixels alone and blends partial coverage', async () => {
    const { tintPixels } = await import('./orbitFrames');
    const rgba = new Uint8ClampedArray([40, 70, 80, 255, 40, 70, 80, 255, 40, 70, 80, 255]);
    tintPixels(rgba, [0, 128, 255], [0.6, 0.15, 0.1], reference);
    expect([...rgba.slice(0, 4)]).toEqual([40, 70, 80, 255]);
    expect(rgba[4]!).toBeGreaterThan(40);
    expect(rgba[4]!).toBeLessThan(rgba[8]!);
    expect(rgba[11]).toBe(255);
  });
});

describe('owner paint sampling', () => {
  function sideArtwork(paint: [number, number, number]) {
    const width = 200, height = 80;
    const rgba = new Uint8ClampedArray(width * height * 4);
    const fill = (x0: number, y0: number, x1: number, y1: number, colour: [number, number, number]) => {
      for (let y = y0; y < y1; y++) for (let x = x0; x < x1; x++) rgba.set([...colour, 255], (y * width + x) * 4);
    };
    fill(10, 10, 190, 70, paint);          // body
    fill(40, 10, 160, 30, [30, 34, 38]);   // glass
    fill(20, 50, 60, 70, [12, 12, 12]);    // tyres
    fill(140, 50, 180, 70, [12, 12, 12]);
    fill(90, 40, 96, 46, [250, 250, 250]); // specular highlight
    return { rgba, width, height };
  }

  it('returns the body paint, not glass, tyres or highlights', async () => {
    const { sampleArtworkPaint, srgbToOklab } = await import('./orbitFrames');
    for (const colour of [[96, 101, 108], [236, 236, 232], [150, 30, 36], [40, 44, 58]] as Array<[number, number, number]>) {
      const { rgba, width, height } = sideArtwork(colour);
      const sampled = sampleArtworkPaint(rgba, width, height)!;
      const expected = srgbToOklab(...colour);
      expect(Math.hypot(sampled[0] - expected[0], sampled[1] - expected[1], sampled[2] - expected[2]) * 100).toBeLessThan(1);
    }
  });

  it('returns null when there is no opaque body to read', async () => {
    const { sampleArtworkPaint } = await import('./orbitFrames');
    expect(sampleArtworkPaint(new Uint8ClampedArray(40 * 20 * 4), 40, 20)).toBeNull();
  });
});

describe('untinted fallback', () => {
  it('does not load masks without an owner paint', async () => {
    const { fetch, images } = stubPacks();
    const { createOrbitFrames } = await import('./orbitFrames');
    const frames = createOrbitFrames();
    frames.setPaint(null);
    await frames.get(56);
    expect(fetch).toHaveBeenCalledOnce();
    expect(String(fetch.mock.calls[0]![0])).not.toContain('mask');
    expect(images).toHaveLength(1);
    frames.close();
  });

  it('shows the frame untinted when the masks cannot be loaded', async () => {
    const { fetch, images } = stubPacks({ masks: false });
    const { createOrbitFrames } = await import('./orbitFrames');
    const frames = createOrbitFrames();
    frames.setPaint([0.6, 0.1, 0.05]);
    const image = await frames.get(56);
    expect(fetch).toHaveBeenCalledTimes(2);
    expect(images).toHaveLength(1);
    expect(image).toBe(images[0]);
    expect(image.close).not.toHaveBeenCalled();
    frames.close();
  });

  it('shows the frame untinted when no canvas is available', async () => {
    const { images } = stubPacks();
    vi.stubGlobal('OffscreenCanvas', class { getContext() { return null; } });
    const { createOrbitFrames } = await import('./orbitFrames');
    const frames = createOrbitFrames();
    frames.setPaint([0.6, 0.1, 0.05]);
    const image = await frames.get(56);
    expect(sourceOf(image)).toBeInstanceOf(Blob);
    expect((sourceOf(image) as Blob).type).toBe('image/webp');
    expect(image.close).not.toHaveBeenCalled();
    expect(images.filter(entry => (entry.source as Blob).type === 'image/png')[0]!.close).toHaveBeenCalled();
    frames.close();
  });

  it('tints on a canvas, caches the tinted frame and drops it when the paint changes', async () => {
    const { images } = stubPacks();
    const puts: ImageData[] = [];
    class FakeCanvas {
      constructor(public width: number, public height: number) {}
      getContext() {
        const { width, height } = this;
        return {
          clearRect() {}, drawImage() {},
          getImageData: (_x: number, _y: number, w: number, h: number) => ({ data: new Uint8ClampedArray(w * h * 4).fill(200), width: w, height: h }),
          putImageData: (data: ImageData) => { puts.push(data); },
          width, height,
        };
      }
    }
    vi.stubGlobal('OffscreenCanvas', FakeCanvas);
    const { createOrbitFrames } = await import('./orbitFrames');
    const frames = createOrbitFrames();
    frames.setPaint([0.6, 0.1, 0.05]);
    const tinted = await frames.get(56);
    expect(sourceOf(tinted)).toBeInstanceOf(FakeCanvas);
    expect(puts).toHaveLength(1);
    const decoded = images.find(entry => (entry.source as Blob).type === 'image/webp')!;
    expect(decoded.close).toHaveBeenCalled();
    expect(await frames.get(56)).toBe(tinted);
    frames.setPaint([0.6, 0.1, 0.05]);
    expect(tinted.close).not.toHaveBeenCalled();
    frames.setPaint(null);
    expect(tinted.close).toHaveBeenCalled();
    const plain = await frames.get(56);
    expect((sourceOf(plain) as Blob).type).toBe('image/webp');
    frames.close();
  });
});

describe('owner-image angle set', () => {
  // The angle set resolver reaches the app through resolveVehicleArtwork(..., 'vehicle-card').angles.
  const angleSet = async () => {
    const { resolveVehicleArtwork } = await import('@panda/hooks');
    return (images: Parameters<typeof resolveVehicleArtwork>[0], design: 'light' | 'dark') =>
      resolveVehicleArtwork(images, 'R2', 'vehicle-card').angles![design];
  };
  const art = (placement: string, design: string | null, url: string, source = 'vehicle:photo') =>
    ({ placement, design, size: 'large', resolution: null, url, metadata: { source } });

  it('orders the owner images front, front three-quarter, side, rear three-quarter, rear and drops overhead and charging', async () => {
    const resolveVehicleAngleSet = await angleSet();
    const images = { all: [
      art('overhead', 'light', '/top.png'), art('side-charging', 'light', '/charging.png'), art('rear', 'light', '/rear.png'),
      art('rear_three_quarter', 'light', '/rq.png'), art('side', 'light', '/side.png'), art('Front Three Quarter', 'light', '/fq.png'),
      art('front', 'light', '/front.png'), art('side', 'light', '/side-small.png'),
    ] };
    expect(resolveVehicleAngleSet(images, 'light')).toEqual([
      { angle: 'front', label: 'Front', url: '/front.png' },
      { angle: 'front-three-quarter', label: 'Front three-quarter', url: '/fq.png' },
      { angle: 'side', label: 'Side', url: '/side.png' },
      { angle: 'rear-three-quarter', label: 'Rear three-quarter', url: '/rq.png' },
      { angle: 'rear', label: 'Rear', url: '/rear.png' },
    ]);
  });

  it('uses the design that matches the theme and keeps one art style, using only placements that exist', async () => {
    const resolveVehicleAngleSet = await angleSet();
    const images = { all: [
      art('side', 'light', '/side-light.png'), art('front', 'light', '/front-light.png'),
      art('side', 'dark', '/side-dark.png'), art('front', 'dark', '/front-dark.png'), art('rear', 'dark', '/rear-dark.png'),
      art('side', 'dark', '/side-cel.png', 'order:cel'),
    ] };
    expect(resolveVehicleAngleSet(images, 'dark').map(angle => angle.url)).toEqual(['/front-dark.png', '/side-dark.png', '/rear-dark.png']);
    expect(resolveVehicleAngleSet(images, 'light').map(angle => angle.url)).toEqual(['/front-light.png', '/side-light.png']);
    // Only the other design exists: use it rather than nothing.
    expect(resolveVehicleAngleSet({ all: [art('side', 'dark', '/s.png'), art('front', 'dark', '/f.png')] }, 'light').map(angle => angle.url))
      .toEqual(['/f.png', '/s.png']);
  });

  it('returns typed pairs when the image list is empty, and nothing without images', async () => {
    const resolveVehicleAngleSet = await angleSet();
    const { resolveVehicleArtwork } = await import('@panda/hooks');
    expect(resolveVehicleAngleSet({ all: [], side: { light: '/side.png', dark: '/side-dark.png' } }, 'dark'))
      .toEqual([{ angle: 'side', label: 'Side', url: '/side-dark.png' }]);
    expect(resolveVehicleAngleSet(null, 'light')).toEqual([]);
    expect(resolveVehicleArtwork({ all: [art('side', 'light', '/s.png')] }, 'R2', 'overview').angles).toEqual({ light: [], dark: [] });
  });
});

describe('owner-image stepping', () => {
  it('wraps, rests on the side and maps keys', async () => {
    const { angleForKey, restAngleIndex, wrapAngleIndex } = await import('./orbitFrames');
    expect(wrapAngleIndex(4, 4)).toBe(0);
    expect(wrapAngleIndex(-1, 4)).toBe(3);
    expect(wrapAngleIndex(1.6, 4)).toBe(2);
    expect(restAngleIndex([{ angle: 'front' }, { angle: 'front-three-quarter' }, { angle: 'side' }])).toBe(2);
    expect(restAngleIndex([{ angle: 'front' }, { angle: 'rear-three-quarter' }])).toBe(1);
    expect(restAngleIndex([{ angle: 'front' }, { angle: 'rear' }])).toBe(0);
    expect(angleForKey('ArrowRight', 3, 4, 2)).toBe(0);
    expect(angleForKey('ArrowLeft', 0, 4, 2)).toBe(3);
    expect(angleForKey('Home', 0, 4, 2)).toBe(2);
    expect(angleForKey('End', 0, 4, 2)).toBe(3);
    expect(angleForKey('a', 0, 4, 2)).toBeNull();
  });

  it('rounds a released drag to the nearest angle with at most one step of inertia', async () => {
    const { ANGLE_INERTIA_MS, angleStepWidth, settleAngle } = await import('./orbitFrames');
    expect(settleAngle(2.3, 0, 4)).toBe(2);
    expect(settleAngle(2.3, 1 / ANGLE_INERTIA_MS, 4)).toBe(3);
    expect(settleAngle(2.3, 100, 4)).toBe(3);
    expect(settleAngle(0.2, -100, 4)).toBe(3);
    expect(angleStepWidth(400)).toBe(100);
    expect(angleStepWidth(100)).toBe(48);
    expect(angleStepWidth(2000)).toBe(140);
  });
});
