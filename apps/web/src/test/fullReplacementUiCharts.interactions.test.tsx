import React from 'react';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { attachChartScrubbing } from '../../../../packages/ui/src/charts/attachChartScrubbing';
import { installMapInspection } from '../../../../packages/ui/src/charts/mapInspection';
import { RichTimeSeriesChart } from '../../../../packages/ui/src/charts/RichTimeSeriesChart';

function pointer(target: Element, type: string, values: Record<string, unknown>) {
  const event = new Event(type, { bubbles: true, cancelable: true });
  Object.assign(event, { pointerId: 1, pointerType: 'touch', isPrimary: true, button: 0, clientX: 20, clientY: 20 }, values);
  fireEvent(target, event);
  return event;
}
afterEach(() => { cleanup(); vi.restoreAllMocks(); });

describe('chart and map interaction contracts', () => {
  it('arbitrates touch scrolling and scrubbing and removes every pointer listener', () => {
    const over = document.createElement('div');
    over.style.touchAction = 'none';
    over.setPointerCapture = vi.fn();
    over.getBoundingClientRect = () => ({ left: 0, top: 0, width: 100, height: 100 } as DOMRect);
    const chart = { over, cursor: { idx: null as number | null }, setCursor: vi.fn(({ left }: { left: number }) => { chart.cursor.idx = Math.floor(left / 10); }) };
    const select = vi.fn();
    const remove = attachChartScrubbing(chart as unknown as Parameters<typeof attachChartScrubbing>[0], select);
    pointer(over, 'pointerdown', {});
    pointer(over, 'pointermove', { clientX: 24, clientY: 23 });
    expect(select).not.toHaveBeenCalled();
    pointer(over, 'pointermove', { clientX: 21, clientY: 40 });
    pointer(over, 'pointerup', { clientX: 80, clientY: 40 });
    expect(select).not.toHaveBeenCalled();
    pointer(over, 'pointerdown', { pointerId: 2 });
    pointer(over, 'pointermove', { pointerId: 2, clientX: 40 });
    expect(select).toHaveBeenLastCalledWith(4);
    pointer(over, 'pointercancel', { pointerId: 2, clientX: 80 });
    expect(select).toHaveBeenCalledTimes(1);
    pointer(over, 'pointermove', { pointerType: 'mouse', clientX: 200 });
    expect(select).toHaveBeenLastCalledWith(10);
    remove();
    expect(over.style.touchAction).toBe('none');
    select.mockClear();
    pointer(over, 'pointermove', { pointerType: 'mouse' });
    expect(select).not.toHaveBeenCalled();
  });

  it('preserves touch-only behavior without hijacking mouse inspection', () => {
    const over = document.createElement('div');
    over.setPointerCapture = vi.fn();
    const chart = { over, cursor: { idx: 1 }, setCursor: vi.fn() };
    const selected = vi.fn();
    const remove = attachChartScrubbing(chart as unknown as Parameters<typeof attachChartScrubbing>[0], selected, true);
    pointer(over, 'pointerdown', { pointerType: 'mouse' });
    pointer(over, 'pointermove', { pointerType: 'mouse' });
    expect(selected).not.toHaveBeenCalled();
    pointer(over, 'pointerdown', {});
    pointer(over, 'pointerup', {});
    expect(selected).toHaveBeenCalledWith(1);
    remove();
  });

  it('releases route inspection for a vertical touch gesture and restores attributes', () => {
    const canvas = document.createElement('canvas');
    canvas.style.cursor = 'grab'; canvas.style.touchAction = 'auto';
    canvas.setPointerCapture = vi.fn(); canvas.hasPointerCapture = () => true; canvas.releasePointerCapture = vi.fn();
    const selected = vi.fn();
    const remove = installMapInspection({ getCanvas: () => canvas, unproject: () => ({ lat: 1, lng: 2 }) }, () => ({ track: [{ lat: 1, lng: 2 }], onPointSelect: selected }));
    pointer(canvas, 'pointerdown', {});
    pointer(canvas, 'pointermove', { clientX: 21, clientY: 40 });
    pointer(canvas, 'pointermove', { clientX: 40, clientY: 40 });
    expect(selected).toHaveBeenCalledTimes(1);
    expect(canvas.releasePointerCapture).toHaveBeenCalledWith(1);
    remove();
    expect(canvas.hasAttribute('aria-label')).toBe(false);
    expect(canvas.style.cursor).toBe('grab');
    expect(canvas.style.touchAction).toBe('auto');
  });

  it('zooms a rich chart by pinch and resets it with the accessible control', async () => {
    vi.spyOn(Date, 'now').mockReturnValue(1_000_000);
    const view = render(<RichTimeSeriesChart points={[{ ts: 0 }, { ts: 50 }, { ts: 100 }]} series={[{ key: 'a', label: 'A', values: [1, 2, 3] }]} xTime={false} interactionMode="touch-explore" />);
    const root = view.container.querySelector('.rich-uplot-chart')!;
    root.getBoundingClientRect = () => ({ left: 0, top: 0, width: 300, height: 200 } as DOMRect);
    await act(async () => {
      pointer(root, 'pointerdown', { pointerId: 1, clientX: 100 });
      pointer(root, 'pointerdown', { pointerId: 2, clientX: 200 });
      pointer(root, 'pointermove', { pointerId: 2, clientX: 250 });
    });
    expect(view.getByRole('button', { name: 'Return to full chart view' })).toBeInTheDocument();
    await act(async () => { fireEvent.click(view.getByRole('button', { name: 'Return to full chart view' })); });
    expect(view.queryByRole('button', { name: 'Return to full chart view' })).not.toBeInTheDocument();
  });
});
