interface Point { lat: number; lng: number }
interface InspectionMap {
  getCanvas(): { style: CSSStyleDeclaration };
  unproject?: (point: [number, number]) => Point;
  dragPan?: { enable(): void; disable(): void };
}
interface InspectionState {
  track: Point[];
  activePoint?: Point | null | undefined;
  activePointIndex?: number | null | undefined;
  onPointSelect?: ((point: Point, trackIndex?: number) => void) | undefined;
}

export function nearestPointIndex(points: Array<{ lat: number | null; lng: number | null }>, target: Point) {
  let nearest: number | null = null;
  let distance = Infinity;
  const longitudeScale = Math.cos(target.lat * Math.PI / 180);
  points.forEach((point, index) => {
    if (point.lat == null || point.lng == null || !Number.isFinite(point.lat) || !Number.isFinite(point.lng)) return;
    const delta = ((point.lng - target.lng + 540) % 360 - 180) * longitudeScale;
    const candidate = (point.lat - target.lat) ** 2 + delta ** 2;
    if (candidate < distance) { distance = candidate; nearest = index; }
  });
  return nearest;
}

export function installMapInspection(map: InspectionMap, state: () => InspectionState) {
  const canvas = map.getCanvas();
  if (!(canvas instanceof HTMLCanvasElement) || !map.unproject || !state().onPointSelect) {
    return Object.assign(() => {}, { setEnabled: (_enabled: boolean) => {} });
  }
  const old = { label: canvas.getAttribute('aria-label'), touch: canvas.style.touchAction, cursor: canvas.style.cursor };
  let enabled = true;
  const setEnabled = (next: boolean) => {
    enabled = next;
    canvas.setAttribute('aria-label', next
      ? 'Inspect trip route. Tap or drag to select a point; arrow keys move along the route.' : 'Pan trip map');
    canvas.style.touchAction = next ? 'pan-y pinch-zoom' : old.touch;
    canvas.style.cursor = next ? 'crosshair' : old.cursor;
    if (next) map.dragPan?.disable(); else map.dragPan?.enable();
  };
  setEnabled(true);
  let pointer: { id: number; x: number; y: number } | null = null;
  const select = (event: PointerEvent) => {
    const bounds = canvas.getBoundingClientRect();
    const point = map.unproject!([event.clientX - bounds.left, event.clientY - bounds.top]);
    if (Number.isFinite(point.lat) && Number.isFinite(point.lng)) state().onPointSelect?.(point);
  };
  const down = (event: PointerEvent) => {
    if (!enabled || !event.isPrimary || event.button !== 0) return;
    pointer = { id: event.pointerId, x: event.clientX, y: event.clientY };
    canvas.setPointerCapture(event.pointerId);
    select(event);
  };
  const move = (event: PointerEvent) => {
    if (!enabled || !pointer || pointer.id !== event.pointerId) return;
    if (event.pointerType === 'touch' && Math.abs(event.clientY - pointer.y) > Math.abs(event.clientX - pointer.x) + 6) {
      up(event); return;
    }
    select(event);
  };
  const up = (event: PointerEvent) => {
    if (pointer?.id !== event.pointerId) return;
    pointer = null;
    if (canvas.hasPointerCapture(event.pointerId)) canvas.releasePointerCapture(event.pointerId);
  };
  const key = (event: KeyboardEvent) => {
    const { track, activePoint, activePointIndex, onPointSelect } = state();
    if (!enabled || !track.length) return;
    const current = activePointIndex != null && activePointIndex >= 0
      ? activePointIndex : activePoint ? nearestPointIndex(track, activePoint) ?? 0 : 0;
    const next = event.key === 'ArrowRight' ? current + 1 : event.key === 'ArrowLeft' ? current - 1
      : event.key === 'Home' ? 0 : event.key === 'End' ? track.length - 1 : null;
    if (next == null) return;
    event.preventDefault();
    event.stopPropagation();
    const index = Math.max(0, Math.min(track.length - 1, next));
    onPointSelect?.(track[index]!, index);
  };
  canvas.addEventListener('pointerdown', down);
  canvas.addEventListener('pointermove', move);
  canvas.addEventListener('pointerup', up);
  canvas.addEventListener('pointercancel', up);
  canvas.addEventListener('keydown', key, true);
  return Object.assign(() => {
    canvas.removeEventListener('pointerdown', down);
    canvas.removeEventListener('pointermove', move);
    canvas.removeEventListener('pointerup', up);
    canvas.removeEventListener('pointercancel', up);
    canvas.removeEventListener('keydown', key, true);
    if (old.label == null) canvas.removeAttribute('aria-label'); else canvas.setAttribute('aria-label', old.label);
    canvas.style.touchAction = old.touch;
    canvas.style.cursor = old.cursor;
    map.dragPan?.enable();
  }, { setEnabled });
}
