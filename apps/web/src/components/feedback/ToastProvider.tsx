import React, { useEffect, useReducer } from 'react';
import type { ToastPayload } from './toast';

let fallbackToastSequence = 0;
let fallbackToastNamespace: string | undefined;

function nextToastId(): string {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') return crypto.randomUUID();
  fallbackToastNamespace ??= `${Date.now()}-${Math.random().toString(36).slice(2)}`;
  fallbackToastSequence += 1;
  return `${fallbackToastNamespace}-${fallbackToastSequence}`;
}

interface Toast extends ToastPayload {
  id: string;
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

type ToastEvent = { type: 'show'; toast: Toast } | { type: 'dismiss'; id: string };

function toastQueue(current: Map<string, Toast>, event: ToastEvent) {
  const next = new Map(current);
  if (event.type === 'dismiss') next.delete(event.id);
  else {
    next.set(event.toast.id, event.toast);
    if (next.size > 3) next.delete(next.keys().next().value!);
  }
  return next;
}

export function ToastProvider({ children }: { children: React.ReactNode }) {
  const [queue, dispatch] = useReducer(toastQueue, new Map<string, Toast>());
  const toasts = Array.from(queue.values());

  useEffect(() => {
    const timers = new Set<number>();
    const receive = (event: Event) => {
      const payload = (event as CustomEvent<ToastPayload>).detail;
      if (!payload?.message) return;
      const toast = { ...payload, id: nextToastId() };
      dispatch({ type: 'show', toast });
      const timer = window.setTimeout(() => {
        timers.delete(timer);
        dispatch({ type: 'dismiss', id: toast.id });
      }, 7000);
      timers.add(timer);
    };
    window.addEventListener('panda:toast', receive);
    return () => {
      window.removeEventListener('panda:toast', receive);
      timers.forEach(timer => window.clearTimeout(timer));
    };
  }, []);

  return (
    <>
      {children}
      <div
        aria-live="polite"
        aria-atomic="true"
        className="fixed right-4 top-4 z-50 flex w-[min(420px,calc(100vw-2rem))] flex-col gap-3"
      >
        {toasts.map(toast => <ToastCard key={toast.id} toast={toast} dismiss={() => dispatch({ type: 'dismiss', id: toast.id })} />)}
      </div>
    </>
  );
}

function ToastCard({ toast, dismiss }: { toast: Toast; dismiss: () => void }) {
  const appearance = TOAST_VARIANT_CLASSNAMES[toast.variant ?? 'default'];
  const paragraphs = new Map([
    ['title', { text: toast.title, className: `text-sm font-semibold leading-6 ${appearance.title}` }],
  ]);
  if (toast.code) paragraphs.set('code', {
    text: toast.code,
    className: 'mt-0.5 font-mono text-[11px] uppercase tracking-[0.14em] text-fg-secondary',
  });
  paragraphs.set('message', { text: toast.message, className: 'mt-1 break-words text-sm leading-6 text-fg' });

  return (
    <div className={`rounded-xl border bg-bg-surface/95 px-4 py-3 text-base text-fg shadow-lg backdrop-blur-md ${appearance.container}`}>
      <div className="flex items-start justify-between gap-3">
        <div>
          {Array.from(paragraphs, ([key, paragraph]) => <p key={key} className={paragraph.className}>{paragraph.text}</p>)}
        </div>
        <button
          type="button"
          aria-label="Dismiss notification"
          className="rounded-md px-2 py-1 text-sm font-medium text-fg-secondary hover:bg-bg-elevated hover:text-fg"
          onClick={dismiss}
        >
          Close
        </button>
      </div>
    </div>
  );
}
