import { readRDataPalette } from './dataPalette';

export function mediaStore(query: string) {
  let media: MediaQueryList | undefined;
  const listeners = new Set<() => void>();
  const notify = () => { for (const listener of listeners) listener(); };
  return {
    snapshot: () => (media ?? window.matchMedia(query)).matches,
    serverSnapshot: () => false,
    subscribe(listener: () => void) {
      if (!media) {
        media = window.matchMedia(query);
        media.addEventListener('change', notify);
      }
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
        if (listeners.size || !media) return;
        media.removeEventListener('change', notify);
        media = undefined;
      };
    },
  };
}

function inputMode(event: Event): 'keyboard' | 'pointer' | null {
  if (event.type === 'pointerdown') return 'pointer';
  const key = event as KeyboardEvent;
  if (key.metaKey || key.ctrlKey || key.altKey) return null;
  if (key.key === 'Tab') return 'keyboard';
  const editing = key.target instanceof Element && key.target.closest('input,textarea,[contenteditable]');
  return editing ? null : 'keyboard';
}

export function attachRDocument() {
  const classes: Array<[Element, string]> = [[document.body, 'r-experience'], [document.documentElement, 'r-interface']];
  for (const [element, name] of classes) element.classList.add(name);
  const palette = readRDataPalette();
  const lifetime = new AbortController();
  const handle = (event: Event) => {
    const mode = inputMode(event);
    if (mode) document.body.dataset.inputMode = mode;
  };
  for (const event of ['pointerdown', 'keydown']) document.addEventListener(event, handle, { capture: true, signal: lifetime.signal });
  return {
    palette,
    dispose() {
      lifetime.abort();
      for (const [element, name] of classes) element.classList.remove(name);
      delete document.body.dataset.inputMode;
    },
  };
}
