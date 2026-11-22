/**
 * Keeps the owner's own vehicle images in this browser, so that a later visit shows the
 * owner's car at once instead of a placeholder.
 *
 * - Image files go to Cache Storage. Only checksum-revisioned mirror files are kept
 *   (`/v1/vehicle-image-cache/{vehicle}/{key}-{16 hex}.{ext}`): a new picture gets a new
 *   address, so a kept file never goes stale. Pending and restoring files are not kept.
 * - The vehicle image list (addresses only, no image data) goes to local storage.
 * - Sign-out clears both.
 * Every call fails soft: without Cache Storage or local storage the app loads images as before.
 */
const ARTWORK_CACHE = 'r-owner-art-v1';
const IMAGES_PREFIX = 'r-owner-images:v1:';
const IMMUTABLE_ARTWORK = /^\/v1\/vehicle-image-cache\/[A-Za-z0-9-]+\/[A-Za-z0-9_.-]+-[0-9a-f]{16}\.[a-z0-9]+$/;

/** True for a checksum-revisioned mirror file, whose content never changes. */
export function isImmutableArtwork(url: string): boolean {
  return IMMUTABLE_ARTWORK.test(url);
}

function cacheStorage(): CacheStorage | null {
  return typeof caches === 'undefined' ? null : caches;
}

/** The kept copy of an owner image, or null. */
export async function readStoredArtwork(url: string): Promise<Blob | null> {
  const storage = cacheStorage();
  if (!storage || !isImmutableArtwork(url)) return null;
  try {
    const response = await (await storage.open(ARTWORK_CACHE)).match(url);
    return response ? await response.blob() : null;
  } catch {
    return null;
  }
}

/** Keeps a complete owner image. Other addresses are ignored. */
export async function storeArtwork(url: string, blob: Blob): Promise<void> {
  const storage = cacheStorage();
  if (!storage || !isImmutableArtwork(url) || blob.size === 0) return;
  try {
    await (await storage.open(ARTWORK_CACHE)).put(url, new Response(blob, {
      headers: { 'Content-Type': blob.type || 'application/octet-stream' },
    }));
  } catch {
    /* Storage is full or blocked: the next visit loads the image again. */
  }
}

/** The kept image list of a vehicle, or undefined. */
export function readStoredVehicleImages<T>(vehicleId: string): T | undefined {
  try {
    const raw = globalThis.localStorage?.getItem(IMAGES_PREFIX + vehicleId);
    return raw ? (JSON.parse(raw) as T) : undefined;
  } catch {
    return undefined;
  }
}

/** Keeps the image list of a vehicle. */
export function storeVehicleImages(vehicleId: string, images: unknown): void {
  try {
    globalThis.localStorage?.setItem(IMAGES_PREFIX + vehicleId, JSON.stringify(images));
  } catch {
    /* Storage is full or blocked. */
  }
}

/** Removes every kept owner image and image list (sign-out). */
export async function clearStoredArtwork(): Promise<void> {
  try {
    const storage = globalThis.localStorage;
    if (storage) {
      const keys: string[] = [];
      for (let index = 0; index < storage.length; index++) {
        const key = storage.key(index);
        if (key?.startsWith(IMAGES_PREFIX)) keys.push(key);
      }
      keys.forEach(key => storage.removeItem(key));
    }
  } catch {
    /* Storage is blocked. */
  }
  try {
    await cacheStorage()?.delete(ARTWORK_CACHE);
  } catch {
    /* Cache Storage is blocked. */
  }
}
