import type uPlot from 'uplot';

export function attachChartScrubbing(chart: uPlot, onSelect: (index: number) => void, touchOnly = false): () => void {
  const target = chart.over;
  const previousTouchAction = target.style.touchAction;
  let pointerId: number | null = null;
  let origin = { x: 0, y: 0 };
  let gesture: 'pending' | 'scrub' | 'scroll' = 'pending';
  target.style.touchAction = 'pan-y';

  const select = (event: PointerEvent) => {
    const bounds = target.getBoundingClientRect();
    chart.setCursor({
      left: Math.max(0, Math.min(bounds.width, event.clientX - bounds.left)),
      top: Math.max(0, Math.min(bounds.height, event.clientY - bounds.top)),
    });
    if (chart.cursor.idx != null && chart.cursor.idx >= 0) onSelect(chart.cursor.idx);
  };
  const start = (event: PointerEvent) => {
    if (touchOnly && event.pointerType !== 'touch') return;
    if (event.button !== 0 || !event.isPrimary) return;
    pointerId = event.pointerId;
    origin = { x: event.clientX, y: event.clientY };
    gesture = event.pointerType === 'mouse' ? 'scrub' : 'pending';
    target.setPointerCapture(pointerId);
    if (gesture === 'scrub') select(event);
  };
  const move = (event: PointerEvent) => {
    if (touchOnly && event.pointerType !== 'touch') return;
    if (event.pointerType === 'mouse') { select(event); return; }
    if (event.pointerId !== pointerId) return;
    if (gesture === 'pending') {
      const dx = Math.abs(event.clientX - origin.x);
      const dy = Math.abs(event.clientY - origin.y);
      if (Math.max(dx, dy) < 8) return;
      gesture = dx > dy ? 'scrub' : 'scroll';
    }
    if (gesture === 'scrub') select(event);
  };
  const end = (event: PointerEvent) => {
    if (event.pointerId !== pointerId) return;
    if (event.type === 'pointerup' && gesture !== 'scroll') select(event);
    pointerId = null;
  };

  target.addEventListener('pointerdown', start);
  target.addEventListener('pointermove', move);
  target.addEventListener('pointerup', end);
  target.addEventListener('pointercancel', end);
  return () => {
    target.style.touchAction = previousTouchAction;
    target.removeEventListener('pointerdown', start);
    target.removeEventListener('pointermove', move);
    target.removeEventListener('pointerup', end);
    target.removeEventListener('pointercancel', end);
  };
}
