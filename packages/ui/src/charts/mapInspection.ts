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
  const scale = Math.cos(target.lat * Math.PI / 180);
  let best: { index: number | null; distance: number } = { index: null, distance: Infinity };
  for (let index = 0; index < points.length; index += 1) {
    const point = points[index]!;
    if (point.lat == null || point.lng == null || !Number.isFinite(point.lat) || !Number.isFinite(point.lng)) continue;
    const latitudeDistance = (point.lat - target.lat) ** 2;
    if (latitudeDistance >= best.distance) continue;
    const longitude = ((point.lng - target.lng + 540) % 360 - 180) * scale;
    const distance = latitudeDistance + longitude ** 2;
    if (distance < best.distance) best = { index, distance };
  }
  return best.index;
}


export function installMapInspection(map: InspectionMap, state: () => InspectionState) {
  const canvas = map.getCanvas();
  if (!(canvas instanceof HTMLCanvasElement) || !map.unproject || !state().onPointSelect) {
    return Object.assign(() => {}, { setEnabled: (_enabled: boolean) => {} });
  }
  const attributes = { label: canvas.getAttribute('aria-label'), touch: canvas.style.touchAction, cursor: canvas.style.cursor };
  let enabled = true;
  let pointer: { id: number; x: number; y: number } | null = null;
  const setEnabled = (next: boolean) => {
    enabled = next;
    const appearance = next
      ? { label: 'Inspect trip route. Tap or drag to select a point; arrow keys move along the route.', touch: 'pan-y pinch-zoom', cursor: 'crosshair' }
      : { label: 'Pan trip map', touch: attributes.touch, cursor: attributes.cursor };
    canvas.setAttribute('aria-label', appearance.label);
    canvas.style.touchAction = appearance.touch;
    canvas.style.cursor = appearance.cursor;
    map.dragPan?.[next ? 'disable' : 'enable']();
  };
  const release = (id: number) => {
    if (pointer?.id !== id) return;
    pointer = null;
    if (canvas.hasPointerCapture(id)) canvas.releasePointerCapture(id);
  };
  const dispatchPointer = (event: PointerEvent) => {
    if (event.type === 'pointerup' || event.type === 'pointercancel') { release(event.pointerId); return; }
    if (!enabled) return;
    if (event.type === 'pointerdown') {
      if (!event.isPrimary || event.button !== 0) return;
      pointer = { id: event.pointerId, x: event.clientX, y: event.clientY };
      canvas.setPointerCapture(event.pointerId);
    } else {
      if (!pointer || pointer.id !== event.pointerId) return;
      const scroll = Math.abs(event.clientY - pointer.y) > Math.abs(event.clientX - pointer.x) + 6;
      if (event.pointerType === 'touch' && scroll) { release(event.pointerId); return; }
    }
    const rect = canvas.getBoundingClientRect();
    const position = map.unproject!([event.clientX - rect.left, event.clientY - rect.top]);
    if (Number.isFinite(position.lat) && Number.isFinite(position.lng)) state().onPointSelect?.(position);
  };
  const key = (event: KeyboardEvent) => {
    const current = state();
    if (!enabled || !current.track.length) return;
    let index = current.activePointIndex != null && current.activePointIndex >= 0
      ? current.activePointIndex : current.activePoint ? nearestPointIndex(current.track, current.activePoint) ?? 0 : 0;
    switch (event.key) {
      case 'ArrowRight': index += 1; break;
      case 'ArrowLeft': index -= 1; break;
      case 'Home': index = 0; break;
      case 'End': index = current.track.length - 1; break;
      default: return;
    }
    event.preventDefault(); event.stopPropagation();
    index = Math.max(0, Math.min(current.track.length - 1, index));
    current.onPointSelect?.(current.track[index]!, index);
  };
  const events = ['pointerdown', 'pointermove', 'pointerup', 'pointercancel'] as const;
  setEnabled(true);
  events.forEach(event => canvas.addEventListener(event, dispatchPointer));
  canvas.addEventListener('keydown', key, true);
  return Object.assign(() => {
    events.forEach(event => canvas.removeEventListener(event, dispatchPointer));
    canvas.removeEventListener('keydown', key, true);
    if (attributes.label == null) canvas.removeAttribute('aria-label'); else canvas.setAttribute('aria-label', attributes.label);
    canvas.style.touchAction = attributes.touch;
    canvas.style.cursor = attributes.cursor;
    map.dragPan?.enable();
  }, { setEnabled });
}
