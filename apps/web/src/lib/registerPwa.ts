/**
 * Registers the static-shell PWA service worker in production builds when supported.
 * Never throws or blocks rendering if registration is unavailable (e.g. private windows).
 */
export function registerPwa() {
  if (typeof window === 'undefined' || !('serviceWorker' in navigator)) return;
  if (!import.meta.env.PROD) return;
  window.addEventListener('load', () => {
    navigator.serviceWorker.register('/sw.js', { scope: '/' }).catch(() => undefined);
  });
}
