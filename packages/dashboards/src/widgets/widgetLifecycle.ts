import React from 'react';

export function useWidgetMediaQuery(query: string, initialWidth?: number) {
  const [matches, setMatches] = React.useState(
    () =>
      typeof window !== 'undefined' &&
      (initialWidth === undefined
        ? window.matchMedia(query).matches
        : window.innerWidth < initialWidth)
  );
  React.useEffect(() => {
    const media = window.matchMedia(query);
    const sync = () => setMatches(media.matches);
    media.addEventListener('change', sync);
    sync();
    return () => {
      media.removeEventListener('change', sync);
    };
  }, [query]);
  return matches;
}

export function useWidgetBounds() {
  const ref = React.useRef<HTMLDivElement | null>(null);
  const [bounds, publish] = React.useReducer(
    (previous: { width: number; height: number }, next: { width: number; height: number }) =>
      previous.width === next.width && previous.height === next.height ? previous : next,
    { width: 0, height: 0 }
  );
  React.useEffect(() => {
    const element = ref.current;
    if (!element || typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(() => {
      const { width, height } = element.getBoundingClientRect();
      publish({ width, height });
    });
    const { width, height } = element.getBoundingClientRect();
    publish({ width, height });
    observer.observe(element);
    return () => {
      observer.disconnect();
    };
  }, []);
  return { ref, ...bounds };
}

export function formatWidgetChargeTime(minutes: number | null | undefined, absent: string) {
  if (minutes == null || minutes <= 0) return absent;
  const parts = [Math.floor(minutes / 60), Math.round(minutes % 60)];
  const units = ['h', 'm'];
  // Zero hours still displays zero minutes for short remaining intervals.
  return parts
    .flatMap((amount, index) =>
      amount !== 0 || (index === 1 && parts[0] === 0) ? [`${amount}${units[index]}`] : []
    )
    .join(' ');
}

export function useWidgetDialogDismiss(
  open: boolean,
  onClose: () => void,
  trigger: React.RefObject<HTMLElement | null>,
  panel: React.RefObject<HTMLElement | null>
) {
  React.useEffect(() => {
    if (!open) return;
    const handlers: Record<string, EventListener> = {
      pointerdown: (event) => {
        if (
          ![trigger.current, panel.current].some((element) =>
            element?.contains(event.target as Node | null)
          )
        )
          onClose();
      },
      keydown: (event) => {
        if ((event as KeyboardEvent).key === 'Escape') onClose();
      },
    };
    Object.entries(handlers).forEach(([name, listener]) =>
      document.addEventListener(name, listener)
    );
    return () =>
      Object.entries(handlers).forEach(([name, listener]) =>
        document.removeEventListener(name, listener)
      );
  }, [open, onClose, trigger, panel]);
}

export function useCompatibleWidgetMedia(query: string) {
  const [matches, publish] = React.useState(
    () =>
      typeof window !== 'undefined' &&
      typeof window.matchMedia === 'function' &&
      window.matchMedia(query).matches
  );
  React.useEffect(() => {
    const media =
      typeof window !== 'undefined' && typeof window.matchMedia === 'function'
        ? window.matchMedia(query)
        : null;
    const sample = () => publish(media?.matches ?? false);
    sample();
    media?.addEventListener?.('change', sample);
    media?.addListener?.(sample);
    return () => {
      media?.removeEventListener?.('change', sample);
      media?.removeListener?.(sample);
    };
  }, [query]);
  return matches;
}
