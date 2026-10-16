import { afterEach, expect, it, vi } from 'vitest';

afterEach(() => { vi.unstubAllGlobals(); vi.resetModules(); });

it('fetches one packed asset on demand and bounds decoded image retention', async () => {
  const fetch = vi.fn(async () => ({ ok: true, arrayBuffer: async () => new ArrayBuffer(3460102) }));
  const images: Array<{ close: ReturnType<typeof vi.fn> }> = [];
  vi.stubGlobal('fetch', fetch);
  vi.stubGlobal('createImageBitmap', vi.fn(async () => {
    const image = { close: vi.fn() }; images.push(image); return image;
  }));
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
