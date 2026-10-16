import frameIndex from './assets/r2-orbit.json';
import packUrl from './assets/r2-orbit.bin?url';

export const ORBIT_FRAMES = 72;
export const DEFAULT_FRAME = 56;
export const FRONT_FRAME = 65;
export const BITMAP_LIMIT = 6;
let pack: Promise<ArrayBuffer> | undefined;

export function wrapFrame(frame: number) {
  return ((Math.round(frame) % ORBIT_FRAMES) + ORBIT_FRAMES) % ORBIT_FRAMES;
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

export function createOrbitFrames() {
  const images = new Map<number, ImageBitmap>();
  const pending = new Map<number, Promise<ImageBitmap>>();
  let disposed = false;
  return {
    async get(frame: number) {
      const index = wrapFrame(frame);
      const cached = images.get(index);
      if (cached) {
        images.delete(index);
        images.set(index, cached);
        return cached;
      }
      const inflight = pending.get(index);
      if (inflight) return inflight;
      const work = (async () => {
        const bytes = await readPack();
        if (disposed) throw new Error('Rotation closed.');
        const [offset, length] = frameIndex[index]!;
        const image = await createImageBitmap(new Blob([bytes.slice(offset, offset! + length!)], { type: 'image/webp' }));
        if (disposed) { image.close(); throw new Error('Rotation closed.'); }
        images.set(index, image);
        while (images.size > BITMAP_LIMIT) {
          const oldest = images.keys().next().value!;
          images.get(oldest)!.close();
          images.delete(oldest);
        }
        return image;
      })().finally(() => pending.delete(index));
      pending.set(index, work);
      return work;
    },
    close() {
      disposed = true;
      images.forEach(image => image.close());
      images.clear();
    },
  };
}
