import { afterEach, expect, it, vi } from 'vitest';

afterEach(() => { vi.unstubAllGlobals(); vi.resetModules(); });
const bytes = () => new ArrayBuffer(3460102);
const deferred = <T,>() => {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(done => { resolve = done; });
  return { promise, resolve };
};

it('deduplicates pending decodes across wrapped indexes and refreshes LRU hits', async () => {
  vi.stubGlobal('fetch', vi.fn(async () => ({ ok: true, arrayBuffer: async () => bytes() })));
  const gate = deferred<ImageBitmap>();
  const first = { close: vi.fn() } as unknown as ImageBitmap;
  const decode = vi.fn().mockReturnValueOnce(gate.promise).mockImplementation(async () => ({ close: vi.fn() }));
  vi.stubGlobal('createImageBitmap', decode);
  const { createOrbitFrames } = await import('../features/r-experience/orbitFrames');
  const cache = createOrbitFrames();
  const one = cache.get(1);
  const wrapped = cache.get(73);
  gate.resolve(first);
  expect(await one).toBe(await wrapped);
  expect(decode).toHaveBeenCalledTimes(1);
  for (let i = 2; i < 7; i++) await cache.get(i);
  await cache.get(1);
  await cache.get(7);
  expect(first.close).not.toHaveBeenCalled();
  await cache.get(2);
  expect(decode).toHaveBeenCalledTimes(8);
  cache.close();
  expect(first.close).toHaveBeenCalledOnce();
  await expect(cache.get(1)).rejects.toThrow('Rotation closed.');
});

it('retries pack failures and rejects invalid pack sizes before decoding', async () => {
  const fetch = vi.fn()
    .mockResolvedValueOnce({ ok: false })
    .mockResolvedValueOnce({ ok: true, arrayBuffer: async () => new ArrayBuffer(2) })
    .mockResolvedValue({ ok: true, arrayBuffer: async () => bytes() });
  const decode = vi.fn(async () => ({ close: vi.fn() }));
  vi.stubGlobal('fetch', fetch);
  vi.stubGlobal('createImageBitmap', decode);
  const { createOrbitFrames } = await import('../features/r-experience/orbitFrames');
  const cache = createOrbitFrames();
  await expect(cache.get(0)).rejects.toThrow('Vehicle rotation could not be loaded.');
  await expect(cache.get(0)).rejects.toThrow('Incomplete vehicle rotation.');
  expect(decode).not.toHaveBeenCalled();
  await cache.get(0);
  expect(fetch).toHaveBeenCalledTimes(3);
  expect(fetch.mock.calls[0]?.[1]).toEqual({ credentials: 'same-origin' });
  cache.close();
});

it('closes a bitmap decoded after disposal exactly once', async () => {
  vi.stubGlobal('fetch', vi.fn(async () => ({ ok: true, arrayBuffer: async () => bytes() })));
  const gate = deferred<ImageBitmap>();
  const decode = vi.fn(() => gate.promise);
  vi.stubGlobal('createImageBitmap', decode);
  const { createOrbitFrames } = await import('../features/r-experience/orbitFrames');
  const cache = createOrbitFrames();
  const pending = cache.get(1);
  await vi.waitFor(() => expect(decode).toHaveBeenCalledOnce());
  cache.close();
  const image = { close: vi.fn() } as unknown as ImageBitmap;
  gate.resolve(image);
  await expect(pending).rejects.toThrow('Rotation closed.');
  cache.close();
  expect(image.close).toHaveBeenCalledOnce();
});
