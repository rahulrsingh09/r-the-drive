import type uPlot from 'uplot';

type Gesture = { pointer: number; x: number; y: number; mode: 'pending' | 'scrub' | 'scroll' };

export function attachChartScrubbing(chart: uPlot, onSelect: (index: number) => void, touchOnly = false) {
  const target = chart.over;
  const savedTouchAction = target.style.touchAction;
  target.style.touchAction = 'pan-y';
  let gesture: Gesture | null = null;
  const inspect = (event: PointerEvent) => {
    const rect = target.getBoundingClientRect();
    const cursor = { left: Math.max(0, Math.min(rect.width, event.clientX - rect.left)), top: Math.max(0, Math.min(rect.height, event.clientY - rect.top)) };
    chart.setCursor(cursor);
    const index = chart.cursor.idx;
    if (index != null && index >= 0) onSelect(index);
  };
  const dispatch = (event: PointerEvent) => {
    switch (event.type) {
      case 'pointerdown':
        if ((touchOnly && event.pointerType !== 'touch') || event.button !== 0 || !event.isPrimary) return;
        gesture = { pointer: event.pointerId, x: event.clientX, y: event.clientY, mode: event.pointerType === 'mouse' ? 'scrub' : 'pending' };
        target.setPointerCapture(event.pointerId);
        if (gesture.mode === 'scrub') inspect(event);
        break;
      case 'pointermove': {
        if (touchOnly && event.pointerType !== 'touch') return;
        if (event.pointerType === 'mouse') { inspect(event); return; }
        if (!gesture || gesture.pointer !== event.pointerId) return;
        const dx = Math.abs(event.clientX - gesture.x);
        const dy = Math.abs(event.clientY - gesture.y);
        if (gesture.mode === 'pending') {
          if (Math.max(dx, dy) < 8) return;
          gesture.mode = dx > dy ? 'scrub' : 'scroll';
        }
        if (gesture.mode === 'scrub') inspect(event);
        break;
      }
      default:
        if (!gesture || gesture.pointer !== event.pointerId) return;
        if (event.type === 'pointerup' && gesture.mode !== 'scroll') inspect(event);
        gesture = null;
    }
  };
  const events = ['pointerdown', 'pointermove', 'pointerup', 'pointercancel'] as const;
  events.forEach(event => target.addEventListener(event, dispatch));
  return () => {
    target.style.touchAction = savedTouchAction;
    events.forEach(event => target.removeEventListener(event, dispatch));
  };
}
