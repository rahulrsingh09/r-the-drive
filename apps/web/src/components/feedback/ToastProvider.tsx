import React, { useCallback, useEffect, useRef, useState } from 'react';
import {
  TOAST_DISMISS_MS,
  TOAST_EVENT,
  addToast,
  toastRole,
  type ToastEntry,
  type ToastPayload,
} from './toast';

// Use crypto.randomUUID() for toast IDs so they remain unique across HMR
// reloads (a module-level counter resets to 0 every time the module is
// re-evaluated, risking duplicate React keys if old toasts are still mounted).
function nextToastId(): string {
  return typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function'
    ? crypto.randomUUID()
    : `${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

const TOAST_VARIANT_CLASSNAMES: Record<NonNullable<ToastPayload['variant']> | 'default', {
  container: string;
  title: string;
}> = {
  default: {
    container: 'border-border-strong',
    title: 'text-fg',
  },
  error: {
    container: 'border-status-danger/70 bg-status-danger/10',
    title: 'text-status-danger',
  },
  warning: {
    container: 'border-status-warning/70 bg-status-warning/10',
    title: 'text-status-warning',
  },
  success: {
    container: 'border-status-positive/70 bg-status-positive/10',
    title: 'text-status-positive',
  },
  info: {
    container: 'border-status-info/70 bg-status-info/10',
    title: 'text-status-info',
  },
};

export function ToastProvider({ children }: { children: React.ReactNode }) {
  const [toasts, setToasts] = useState<ToastEntry[]>([]);

  useEffect(() => {
    function handleToast(event: Event) {
      const detail = (event as CustomEvent<ToastPayload>).detail;
      if (!detail?.message) return;
      setToasts((current) => addToast(current, detail, nextToastId));
    }

    window.addEventListener(TOAST_EVENT, handleToast);
    return () => window.removeEventListener(TOAST_EVENT, handleToast);
  }, []);

  const dismiss = useCallback(
    (id: string) => setToasts((current) => current.filter((toast) => toast.id !== id)),
    [],
  );

  return (
    <>
      {children}
      {/* One permanent polite live region. It exists before the first toast, so
          screen readers announce success and info toasts as they are added. */}
      <section
        aria-label="Notifications"
        aria-live="polite"
        aria-relevant="additions text"
        className="fixed right-4 top-4 z-50 flex w-[min(420px,calc(100vw-2rem))] flex-col gap-3"
      >
        {toasts.map((toast) => (
          <ToastItem key={toast.id} toast={toast} onDismiss={dismiss} />
        ))}
      </section>
    </>
  );
}

function ToastItem({ toast, onDismiss }: { toast: ToastEntry; onDismiss: (id: string) => void }) {
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  const paused = hovered || focused;
  const remainingMs = useRef(TOAST_DISMISS_MS);

  // A merged duplicate restarts the full display time.
  useEffect(() => {
    remainingMs.current = TOAST_DISMISS_MS;
  }, [toast.count]);

  useEffect(() => {
    if (paused) return;
    const startedAt = Date.now();
    const timer = window.setTimeout(() => onDismiss(toast.id), remainingMs.current);
    return () => {
      window.clearTimeout(timer);
      remainingMs.current = Math.max(0, remainingMs.current - (Date.now() - startedAt));
    };
  }, [paused, toast.id, toast.count, onDismiss]);

  const variant = TOAST_VARIANT_CLASSNAMES[toast.variant ?? 'default'];
  return (
    <div
      role={toastRole(toast.variant)}
      aria-atomic="true"
      data-toast-count={toast.count}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      onFocus={() => setFocused(true)}
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setFocused(false);
      }}
      className={`rounded-xl border bg-bg-surface/95 px-4 py-3 text-base text-fg shadow-lg backdrop-blur-md ${variant.container}`}
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <p className={`text-sm font-semibold leading-6 ${variant.title}`}>
            {toast.title}
            {toast.count > 1 && (
              <>
                <span
                  aria-hidden="true"
                  className="ml-2 rounded-full border border-border-strong px-2 py-0.5 font-mono text-[11px] text-fg-secondary"
                >
                  {toast.count}×
                </span>
                <span className="sr-only">{`, shown ${toast.count} times`}</span>
              </>
            )}
          </p>
          {toast.code && (
            <p className="mt-0.5 font-mono text-[11px] uppercase tracking-[0.14em] text-fg-secondary">
              {toast.code}
            </p>
          )}
          <p className="mt-1 break-words text-sm leading-6 text-fg">{toast.message}</p>
        </div>
        <button
          type="button"
          aria-label="Dismiss notification"
          className="inline-flex min-h-11 min-w-11 items-center justify-center rounded-md px-2 text-sm font-medium text-fg-secondary hover:bg-bg-elevated hover:text-fg"
          onClick={() => onDismiss(toast.id)}
        >
          Close
        </button>
      </div>
    </div>
  );
}
