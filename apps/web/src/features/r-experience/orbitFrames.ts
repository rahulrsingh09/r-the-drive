import frameIndex from './assets/r2-orbit.json';
import packUrl from './assets/r2-orbit.bin?url';

export const ORBIT_FRAMES = 72;
export const DEFAULT_FRAME = 56;
export const FRONT_FRAME = 65;
export const BITMAP_LIMIT = 6;

const packedAsset: { bytes?: ArrayBuffer; request?: Promise<ArrayBuffer> } = {};

export function wrapFrame(frame: number) {
  return ((Math.round(frame) % ORBIT_FRAMES) + ORBIT_FRAMES) % ORBIT_FRAMES;
}

async function readPack(): Promise<ArrayBuffer> {
  if (packedAsset.bytes) return packedAsset.bytes;
  if (packedAsset.request) return packedAsset.request;
  const request = fetch(packUrl, { credentials: 'same-origin' }).then(async response => {
    if (!response.ok) throw new Error('Vehicle rotation could not be loaded.');
    const buffer = await response.arrayBuffer();
    const end = frameIndex[frameIndex.length - 1]!;
    if (buffer.byteLength !== end[0]! + end[1]!) throw new Error('Incomplete vehicle rotation.');
    packedAsset.bytes = buffer;
    return buffer;
  });
  packedAsset.request = request;
  try {
    return await request;
  } finally {
    delete packedAsset.request;
  }
}

type DecodedFrame = { index: number; bitmap: ImageBitmap; touched: number };

class OrbitFrameStore {
  private decoded: DecodedFrame[] = [];
  private requests = new Map<number, Promise<ImageBitmap>>();
  private closed = false;
  private clock = 0;

  async get(frame: number): Promise<ImageBitmap> {
    const index = wrapFrame(frame);
    const hit = this.decoded.find(item => item.index === index);
    if (hit) {
      hit.touched = ++this.clock;
      return hit.bitmap;
    }
    if (this.requests.has(index)) return this.requests.get(index)!;
    const request = this.decode(index);
    this.requests.set(index, request);
    try {
      return await request;
    } finally {
      this.requests.delete(index);
    }
  }

  private async decode(index: number) {
    const buffer = await readPack();
    this.assertOpen();
    const [offset, length] = frameIndex[index]!;
    const blob = new Blob([new Uint8Array(buffer, offset, length)], { type: 'image/webp' });
    const bitmap = await createImageBitmap(blob);
    if (this.closed) {
      bitmap.close();
      this.assertOpen();
    }
    this.decoded.push({ index, bitmap, touched: ++this.clock });
    if (this.decoded.length > BITMAP_LIMIT) {
      const oldest = this.decoded.reduce((a, b) => a.touched < b.touched ? a : b);
      this.decoded.splice(this.decoded.indexOf(oldest), 1);
      oldest.bitmap.close();
    }
    return bitmap;
  }

  private assertOpen() {
    if (this.closed) throw new Error('Rotation closed.');
  }

  close() {
    this.closed = true;
    const held = this.decoded;
    this.decoded = [];
    for (const item of held) item.bitmap.close();
  }
}

export function createOrbitFrames() {
  return new OrbitFrameStore();
}
