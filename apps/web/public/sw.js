/* Privacy-preserving PWA service worker for R:The Drive.
 * Never caches /v1/*, /v2/* API responses or telemetry streams; only caches
 * immutable static shell assets and provides an offline navigation fallback. */
const SHELL_CACHE = 'r-shell-v1';
const PRECACHE_URLS = ['/', '/manifest.webmanifest', '/r-icon.svg'];

self.addEventListener('install', (event) => {
  event.waitUntil(
    caches.open(SHELL_CACHE)
      .then((cache) => cache.addAll(PRECACHE_URLS).catch(() => undefined))
      .then(() => self.skipWaiting()),
  );
});

self.addEventListener('activate', (event) => {
  event.waitUntil(
    caches.keys()
      .then((keys) => Promise.all(
        keys.filter((key) => key !== SHELL_CACHE).map((key) => caches.delete(key)),
      ))
      .then(() => self.clients.claim()),
  );
});

self.addEventListener('fetch', (event) => {
  const { request } = event;
  if (request.method !== 'GET') return;

  const url = new URL(request.url);
  if (url.origin !== self.location.origin) return;
  if (url.pathname.startsWith('/v1/') || url.pathname.startsWith('/v2/')) return;

  if (request.mode === 'navigate') {
    event.respondWith(
      fetch(request)
        .then((response) => {
          if (response && response.ok && response.type === 'basic') {
            const copy = response.clone();
            caches.open(SHELL_CACHE).then((cache) => cache.put('/', copy)).catch(() => undefined);
          }
          return response;
        })
        .catch(() => caches.match('/') || caches.match(request)),
    );
    return;
  }

  const isStaticAsset = url.pathname.startsWith('/assets/')
    || url.pathname.startsWith('/fonts/')
    || url.pathname === '/r-icon.svg'
    || url.pathname === '/manifest.webmanifest';

  if (!isStaticAsset) return;

  event.respondWith(
    caches.match(request).then((cached) => {
      if (cached) return cached;
      return fetch(request).then((response) => {
        if (response && response.ok && response.type === 'basic') {
          const copy = response.clone();
          caches.open(SHELL_CACHE).then((cache) => cache.put(request, copy)).catch(() => undefined);
        }
        return response;
      });
    }),
  );
});
