import type { ChangeEvent, FocusEvent, MouseEvent, KeyboardEvent, RefObject } from 'react';

export function numericInput(change: (value: number) => void) {
  return (event: ChangeEvent<HTMLInputElement> | string) => {
    const raw = typeof event === 'string' ? event : event.target.value;
    change(Number(raw));
  };
}
export function backdropAction(action: () => void) {
  return (event: MouseEvent) => {
    if (event.currentTarget !== event.target) return;
    action();
  };
}
export function keyAction(key: string, action: () => void) {
  return (event: KeyboardEvent) => {
    if (event.key !== key) return;
    action();
  };
}
export function leaveFocus(action: () => void) {
  return (event: FocusEvent<HTMLElement>) => {
    const next = event.relatedTarget;
    if (next && event.currentTarget.contains(next as Node)) return;
    action();
  };
}
export function selectionAction<T>(change: (value: T) => void, close: () => void, trigger?: RefObject<HTMLElement | null>, preventScroll?: boolean) {
  return (value: T) => {
    change(value);
    close();
    const target = trigger?.current;
    if (!target) return;
    if (preventScroll === undefined) target.focus();
    else target.focus({ preventScroll });
  };
}
