/** Browser resources are acquired together and released in reverse order. */
export function listen(
  registrations: ReadonlyArray<readonly [EventTarget, string, EventListener, boolean?]>,
): () => void {
  const releases: Array<() => void> = [];
  for (const [target, type, handler, capture = false] of registrations) {
    target.addEventListener(type, handler, capture);
    releases.push(() => target.removeEventListener(type, handler, capture));
  }
  return () => { while (releases.length) releases.pop()!(); };
}

export function browserStorage<T>(fallback: T, operation: (storage: Storage, browser: Window) => T): T {
  try {
    return typeof window === 'undefined' ? fallback : operation(window.localStorage, window);
  } catch {
    return fallback;
  }
}

export function lockBodyScroll(): () => void {
  const body = document.body;
  const overflow = body.style.overflow;
  body.style.overflow = 'hidden';
  return () => { body.style.overflow = overflow; };
}

export function readMedia(query: string): MediaQueryList | null {
  return typeof window === 'undefined' || typeof window.matchMedia !== 'function' ? null : window.matchMedia(query);
}

export function observeMedia(media: MediaQueryList | null, changed: () => void): () => void {
  if (!media) return () => undefined;
  if (typeof media.addEventListener === 'function') return listen([[media, 'change', changed]]);
  media.addListener?.(changed);
  return () => media.removeListener?.(changed);
}
