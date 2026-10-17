import React from 'react';

type Viewport = { portrait: boolean; height: number };
type ViewerState = Viewport & { controls: boolean };
type ViewerAction =
  { type: 'viewport'; viewport: Viewport } | { type: 'toggle' } | { type: 'show' | 'hide' };
function viewport(): Viewport {
  if (typeof window === 'undefined') return { portrait: false, height: 320 };
  return {
    portrait: window.matchMedia('(orientation: portrait)').matches,
    height: Math.max(240, Math.floor((window.visualViewport?.height ?? window.innerHeight) - 16)),
  };
}
function viewerState(state: ViewerState, action: ViewerAction): ViewerState {
  if (action.type === 'viewport') return { ...state, ...action.viewport };
  const controls = action.type === 'toggle' ? !state.controls : action.type === 'show';
  return controls === state.controls ? state : { ...state, controls };
}
function freezePage() {
  const scrollY = window.scrollY;
  const patches: Array<[HTMLElement, Partial<CSSStyleDeclaration>]> = [
    [document.documentElement, { overflow: 'hidden', overscrollBehavior: 'none' }],
    [
      document.body,
      {
        overflow: 'hidden',
        overscrollBehavior: 'none',
        position: 'fixed',
        top: `-${scrollY}px`,
        width: '100%',
      },
    ],
  ];
  const restore = patches.map(([element, patch]) => {
    const saved: Record<string, string> = {};
    for (const key of Object.keys(patch))
      saved[key] = element.style[key as keyof CSSStyleDeclaration] as string;
    Object.assign(element.style, patch);
    return () => Object.assign(element.style, saved);
  });
  const app = document.getElementById('root'),
    inert = app?.inert;
  if (app) app.inert = true;
  return () => {
    restore.forEach((restoreStyle) => restoreStyle());
    if (app) app.inert = inert ?? false;
    if (window.scrollY !== scrollY) window.scrollTo(0, scrollY);
  };
}

type Tap = { id: number; x: number; y: number; valid: boolean };
const INTERACTIVE =
  'button, input, select, textarea, a, [role="option"], [data-mobile-chart-controls="true"]';
function interactive(target: EventTarget | null) {
  return target instanceof Element && target.closest(INTERACTIVE) !== null;
}

export function useMobileChartController(onClose: () => void) {
  const rootRef = React.useRef<HTMLDivElement | null>(null);
  const closeRef = React.useRef<HTMLButtonElement | null>(null);
  const closed = React.useRef(false),
    tap = React.useRef<Tap | null>(null);
  const [state, dispatch] = React.useReducer(viewerState, undefined, () => ({
    ...viewport(),
    controls: true,
  }));
  const close = React.useCallback(() => {
    if (closed.current) return;
    closed.current = true;
    const ownFullscreen = document.fullscreenElement === rootRef.current;
    if (ownFullscreen) void document.exitFullscreen?.().catch(() => undefined);
    try {
      screen.orientation?.unlock?.();
    } catch {
      /* Optional browser capability. */
    }
    onClose();
  }, [onClose]);
  const hide = React.useCallback(() => {
    dispatch({ type: 'hide' });
    rootRef.current?.focus();
  }, []);
  React.useEffect(() => {
    const query = window.matchMedia('(orientation: portrait)');
    const sample = () => dispatch({ type: 'viewport', viewport: viewport() });
    const registrations: Array<[EventTarget, string]> = [
      [query, 'change'],
      [window, 'resize'],
    ];
    registrations.forEach(([target, event]) => target.addEventListener(event, sample));
    sample();
    return () =>
      registrations.forEach(([target, event]) => target.removeEventListener(event, sample));
  }, []);
  React.useEffect(() => {
    if (state.portrait) {
      dispatch({ type: 'show' });
      return;
    }
    const timeout = window.setTimeout(hide, 1000);
    return () => window.clearTimeout(timeout);
  }, [state.portrait, hide]);
  React.useEffect(() => {
    const restorePage = freezePage();
    closeRef.current?.focus();
    const events = {
      keydown: (event: Event) => {
        if ((event as KeyboardEvent).key === 'Escape') close();
      },
      fullscreenchange: () => {
        if (!closed.current && document.fullscreenElement == null) close();
      },
    };
    Object.entries(events).forEach(([name, listener]) => document.addEventListener(name, listener));
    async function expand() {
      const root = rootRef.current;
      if (!root?.requestFullscreen) return;
      try {
        await root.requestFullscreen();
        await screen.orientation?.lock?.('landscape');
      } catch {
        /* Fullscreen and orientation may be unavailable. */
      }
    }
    void expand();
    return () => {
      restorePage();
      Object.entries(events).forEach(([name, listener]) =>
        document.removeEventListener(name, listener)
      );
    };
  }, [close]);
  function pointer(phase: 'down' | 'move' | 'up', event: React.PointerEvent<HTMLDivElement>) {
    const current = tap.current;
    if (phase === 'down') {
      if (state.portrait || interactive(event.target)) return;
      tap.current = current
        ? { ...current, valid: false }
        : { id: event.pointerId, x: event.clientX, y: event.clientY, valid: true };
      return;
    }
    if (!current || current.id !== event.pointerId) return;
    if (phase === 'move') {
      if (Math.hypot(event.clientX - current.x, event.clientY - current.y) > 12)
        current.valid = false;
      return;
    }
    tap.current = null;
    if (current.valid && !interactive(event.target)) dispatch({ type: 'toggle' });
  }
  return {
    rootRef,
    closeRef,
    close,
    isPortrait: state.portrait,
    chartHeight: state.height,
    showControls: state.controls,
    onViewerPointerDownCapture: (event: React.PointerEvent<HTMLDivElement>) =>
      pointer('down', event),
    onViewerPointerMoveCapture: (event: React.PointerEvent<HTMLDivElement>) =>
      pointer('move', event),
    onViewerPointerUpCapture: (event: React.PointerEvent<HTMLDivElement>) => pointer('up', event),
    cancelPointer: () => {
      tap.current = null;
    },
    onViewerKeyDown: (event: React.KeyboardEvent<HTMLDivElement>) => {
      if (['Enter', ' '].includes(event.key)) {
        event.preventDefault();
        dispatch({ type: 'toggle' });
      }
    },
  };
}
