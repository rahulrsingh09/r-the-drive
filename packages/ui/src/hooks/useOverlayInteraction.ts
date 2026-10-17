import * as React from 'react';
import { listen, lockBodyScroll, observeMedia, readMedia } from '../lib/browserRuntime';

export function useDisclosure(initial = false) {
  const [open, dispatch] = React.useReducer((current: boolean, action: React.SetStateAction<boolean>) =>
    typeof action === 'function' ? action(current) : action, initial);
  const show = React.useCallback(() => dispatch(true), []);
  const hide = React.useCallback(() => dispatch(false), []);
  const toggle = React.useCallback(() => dispatch(current => !current), []);
  return { open, setOpen: dispatch, show, hide, toggle };
}

type ElementRef = React.RefObject<HTMLElement | null>;
export function useDismissLayer({ enabled = true, roots, dismiss, pointer = 'pointerdown', captureEscape = false, preventEscape = false, requireRoot = false }: {
  enabled?: boolean; roots: ElementRef[]; dismiss: (reason: 'outside' | 'escape') => void;
  pointer?: 'pointerdown' | 'mousedown'; captureEscape?: boolean; preventEscape?: boolean; requireRoot?: boolean;
}) {
  React.useEffect(() => {
    if (!enabled) return;
    const handle: EventListener = event => {
      if (event.type === 'keydown') {
        if ((event as KeyboardEvent).key !== 'Escape') return;
        if (preventEscape) event.preventDefault();
        if (captureEscape) event.stopPropagation();
        dismiss('escape');
        return;
      }
      if (requireRoot && !roots.some(ref => ref.current)) return;
      if (roots.some(ref => ref.current?.contains(event.target as Node | null))) return;
      dismiss('outside');
    };
    return listen([[document, pointer, handle], [document, 'keydown', handle, captureEscape]]);
  }, [enabled, roots, dismiss, pointer, captureEscape, preventEscape, requireRoot]);
}

const FOCUSABLE = 'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';
export function useDialogLifecycle(dialog: ElementRef, onClose: () => void) {
  const close = React.useRef(onClose);
  React.useEffect(() => { close.current = onClose; }, [onClose]);
  const previousFocus = React.useRef<HTMLElement | null>(typeof document !== 'undefined' && document.activeElement instanceof HTMLElement ? document.activeElement : null);
  React.useEffect(() => {
    const unlock = lockBodyScroll();
    dialog.current?.querySelector<HTMLElement>(FOCUSABLE)?.focus();
    const keyboard: EventListener = event => {
      const key = event as KeyboardEvent;
      if (key.key === 'Escape') { close.current(); return; }
      if (key.key !== 'Tab') return;
      const items = dialog.current?.querySelectorAll<HTMLElement>(FOCUSABLE);
      if (!items?.length) return;
      const edge = key.shiftKey ? 0 : items.length - 1;
      if (document.activeElement !== items[edge]) return;
      key.preventDefault();
      items[key.shiftKey ? items.length - 1 : 0]?.focus();
    };
    const unlisten = listen([[document, 'keydown', keyboard]]);
    return () => { unlisten(); unlock(); previousFocus.current?.focus(); };
  }, [dialog]);
}

export function useViewportPlacement(enabled: boolean, measure: () => void, visualViewport = false) {
  React.useLayoutEffect(() => {
    if (!enabled) return;
    measure();
    const registrations: Array<readonly [EventTarget, string, EventListener, boolean?]> = [
      [window, 'resize', measure], [window, 'scroll', measure, true],
    ];
    if (visualViewport && window.visualViewport) {
      registrations.push([window.visualViewport, 'resize', measure], [window.visualViewport, 'scroll', measure]);
    }
    return listen(registrations);
  }, [enabled, measure, visualViewport]);
}

export function useMediaQuery(query: string) {
  const subscribe = React.useCallback((notify: () => void) => observeMedia(readMedia(query), notify), [query]);
  const snapshot = React.useCallback(() => readMedia(query)?.matches ?? false, [query]);
  return React.useSyncExternalStore(subscribe, snapshot, () => false);
}
