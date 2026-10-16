// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { installMapInspection, nearestPointIndex } from './mapInspection';

afterEach(() => vi.restoreAllMocks());

describe('map inspection', () => {
  it('finds a recorded sample and ignores missing coordinates', () => {
    expect(nearestPointIndex([{ lat: null, lng: null }, { lat: 47, lng: -122 }, { lat: 48, lng: -123 }], { lat: 47.1, lng: -122 })).toBe(1);
    expect(nearestPointIndex([{ lat: NaN, lng: 1 }], { lat: 47, lng: -122 })).toBeNull();
    expect(nearestPointIndex([{ lat: 0, lng: -179.9 }, { lat: 0, lng: 170 }], { lat: 0, lng: 179.9 })).toBe(0);
  });
  it('selects points from the whole canvas and restores map behavior on cleanup', () => {
    const canvas = document.createElement('canvas');
    canvas.setAttribute('aria-label', 'Map');
    canvas.setPointerCapture = vi.fn();
    canvas.releasePointerCapture = vi.fn();
    canvas.hasPointerCapture = () => true;
    const onPointSelect = vi.fn();
    const map = { getCanvas: () => canvas, unproject: vi.fn(() => ({ lat: 47, lng: -122 })), dragPan: { enable: vi.fn(), disable: vi.fn() } };
    const remove = installMapInspection(map, () => ({ track: [{ lat: 47, lng: -122 }], onPointSelect }));
    const event = new Event('pointerdown');
    Object.assign(event, { pointerId: 1, isPrimary: true, button: 0, clientX: 70, clientY: 90 });
    canvas.dispatchEvent(event);
    expect(map.unproject).toHaveBeenCalledWith([70, 90]);
    expect(onPointSelect).toHaveBeenCalledWith({ lat: 47, lng: -122 });
    expect(map.dragPan.disable).toHaveBeenCalledOnce();
    remove();
    onPointSelect.mockClear();
    canvas.dispatchEvent(event);
    expect(onPointSelect).not.toHaveBeenCalled();
    expect(canvas.getAttribute('aria-label')).toBe('Map');
    expect(map.dragPan.enable).toHaveBeenCalledOnce();
  });
  it('supports bounded keyboard selection without intercepting unrelated keys', () => {
    const canvas = document.createElement('canvas');
    const track = [{ lat: 1, lng: 2 }, { lat: 3, lng: 4 }];
    const onPointSelect = vi.fn();
    const remove = installMapInspection({ getCanvas: () => canvas, unproject: () => track[0]! }, () => ({ track, onPointSelect }));
    canvas.dispatchEvent(new KeyboardEvent('keydown', { key: 'End' }));
    expect(onPointSelect).toHaveBeenLastCalledWith(track[1], 1);
    canvas.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowLeft' }));
    expect(onPointSelect).toHaveBeenLastCalledWith(track[0], 0);
    onPointSelect.mockClear();
    canvas.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab' }));
    expect(onPointSelect).not.toHaveBeenCalled();
    remove();
  });
  it('steps through repeated GPS samples by index and can return to map panning', () => {
    const canvas = document.createElement('canvas');
    const track = [{ lat: 1, lng: 2 }, { lat: 1, lng: 2 }, { lat: 3, lng: 4 }];
    let activePointIndex = 0;
    const onPointSelect = vi.fn((_point, index: number | undefined) => { activePointIndex = index!; });
    const dragPan = { enable: vi.fn(), disable: vi.fn() };
    const control = installMapInspection({ getCanvas: () => canvas, unproject: () => track[0]!, dragPan },
      () => ({ track, activePoint: track[activePointIndex], activePointIndex, onPointSelect }));
    canvas.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight' }));
    canvas.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight' }));
    expect(onPointSelect.mock.calls.map(call => call[1])).toEqual([1, 2]);
    control.setEnabled(false);
    expect(dragPan.enable).toHaveBeenCalledOnce();
    onPointSelect.mockClear();
    canvas.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowLeft' }));
    expect(onPointSelect).not.toHaveBeenCalled();
    control();
  });
});
