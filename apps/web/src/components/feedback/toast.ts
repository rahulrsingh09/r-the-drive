export type ToastVariant = 'error' | 'warning' | 'success' | 'info';

export interface ToastPayload {
  title: string;
  message: string;
  code?: string;
  variant?: ToastVariant;
}

export interface ToastEntry extends ToastPayload {
  id: string;
  /** Number of identical toasts merged into this entry. */
  count: number;
}

export const TOAST_EVENT = 'panda:toast';
/** Auto-dismiss time. Hover or keyboard focus pauses it. */
export const TOAST_DISMISS_MS = 7000;
export const MAX_VISIBLE_TOASTS = 3;

/** Errors interrupt assistive technology; other variants wait politely. */
export function toastRole(variant: ToastVariant | undefined): 'alert' | 'status' {
  return variant === 'error' ? 'alert' : 'status';
}

function sameToast(a: ToastPayload, b: ToastPayload) {
  return a.title === b.title && a.message === b.message && (a.variant ?? null) === (b.variant ?? null);
}

/**
 * Adds a toast to the visible stack. A duplicate (same title, message, and
 * variant) merges into the existing entry, raises its count, and moves it to
 * the newest position. The stack keeps the newest MAX_VISIBLE_TOASTS entries.
 */
export function addToast(
  current: ToastEntry[],
  payload: ToastPayload,
  nextId: () => string,
): ToastEntry[] {
  const existing = current.find((toast) => sameToast(toast, payload));
  const entry: ToastEntry = existing
    ? { ...existing, ...payload, count: existing.count + 1 }
    : { ...payload, id: nextId(), count: 1 };
  const rest = current.filter((toast) => toast !== existing);
  return [...rest, entry].slice(-MAX_VISIBLE_TOASTS);
}

export function emitToast(payload: ToastPayload) {
  window.dispatchEvent(new CustomEvent<ToastPayload>(TOAST_EVENT, { detail: payload }));
}

export function emitAuthError(title: string, message: string) {
  emitToast({ title, message, variant: 'error' });
}
