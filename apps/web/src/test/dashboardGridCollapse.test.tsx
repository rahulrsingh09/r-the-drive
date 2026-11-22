import React from 'react';
import { act, render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { DashboardGrid, collapseHiddenWidgets, rowsForHeight } from '../../../../packages/dashboards/src/DashboardGrid';
import type { WidgetInstance } from '../../../../packages/dashboards/src/schema';

const emptyIds = new Set<string>();
const setEmpty = (id: string, empty: boolean) => (empty ? emptyIds.add(id) : emptyIds.delete(id));
const emptyCharts = new Set<string>();
const listeners = new Set<() => void>();

vi.mock('../../../../packages/dashboards/src/WidgetHost', async () => {
  const ReactModule = await import('react');
  function FakeHost({ instance }: { instance: { id: string; componentType: string } }) {
    const [, force] = ReactModule.useReducer((count: number) => count + 1, 0);
    ReactModule.useEffect(() => {
      listeners.add(force);
      return () => { listeners.delete(force); };
    }, []);
    if (instance.componentType === 'chart') {
      return emptyCharts.has(instance.id)
        ? <div role="status" data-chart-empty-row="true">No data</div>
        : <svg data-testid={`chart-${instance.id}`} />;
    }
    return emptyIds.has(instance.id)
      ? <div hidden aria-hidden="true" data-sensor-empty="hidden" />
      : <div data-testid={`tile-${instance.id}`}>{instance.id}</div>;
  }
  return { WidgetHost: FakeHost };
});

function widget(id: string, x: number, y: number, w: number, h = 2, componentType: WidgetInstance['componentType'] = 'sensor'): WidgetInstance {
  return { id, componentType, definitionId: `def-${id}`, layout: { x, y, w, h }, options: {} };
}

const TILE_ROW = [widget('a', 0, 0, 3), widget('b', 3, 0, 3), widget('c', 6, 0, 3), widget('d', 9, 0, 3)];
const CHART = widget('chart', 0, 2, 12, 6, 'custom');

describe('collapseHiddenWidgets', () => {
  it('keeps the saved layout when no widget is hidden', () => {
    const layouts = collapseHiddenWidgets([...TILE_ROW, CHART], new Set());
    expect(layouts.get('b')).toBe(TILE_ROW[1]!.layout);
    expect(layouts.get('chart')).toBe(CHART.layout);
  });

  it('lets the other tiles of a row share the width of a hidden tile', () => {
    const layouts = collapseHiddenWidgets([...TILE_ROW, CHART], new Set(['b']));
    expect(layouts.has('b')).toBe(false);
    expect(['a', 'c', 'd'].map((id) => layouts.get(id))).toEqual([
      { x: 0, y: 0, w: 4, h: 2 },
      { x: 4, y: 0, w: 4, h: 2 },
      { x: 8, y: 0, w: 4, h: 2 },
    ]);
    expect(layouts.get('chart')).toEqual(CHART.layout);
  });

  it('keeps the ratio of unequal widths', () => {
    const row = [widget('a', 0, 0, 6), widget('b', 6, 0, 3), widget('c', 9, 0, 3)];
    const layouts = collapseHiddenWidgets(row, new Set(['c']));
    expect(layouts.get('a')).toEqual({ x: 0, y: 0, w: 8, h: 2 });
    expect(layouts.get('b')).toEqual({ x: 8, y: 0, w: 4, h: 2 });
  });

  it('moves widgets up when a full row hides', () => {
    const layouts = collapseHiddenWidgets([...TILE_ROW, CHART], new Set(['a', 'b', 'c', 'd']));
    expect(layouts.get('chart')).toEqual({ x: 0, y: 0, w: 12, h: 6 });
  });

  it('gives an empty chart row its smaller height and moves the widgets below up', () => {
    const chart = widget('chart', 0, 0, 12, 10, 'chart');
    const below = widget('below', 0, 10, 12, 4, 'custom');
    const layouts = collapseHiddenWidgets([chart, below], new Set(), new Map([['chart', 2]]));
    expect(layouts.get('chart')).toEqual({ x: 0, y: 0, w: 12, h: 2 });
    expect(layouts.get('below')).toEqual({ x: 0, y: 2, w: 12, h: 4 });
    // A larger height never grows the saved cell.
    expect(collapseHiddenWidgets([chart], new Set(), new Map([['chart', 12]])).get('chart')).toBe(chart.layout);
  });

  it('converts a pixel height to desktop rows', () => {
    expect(rowsForHeight(0)).toBe(1);
    expect(rowsForHeight(40)).toBe(1);
    expect(rowsForHeight(64)).toBe(2);
    expect(rowsForHeight(96)).toBe(2);
    expect(rowsForHeight(97)).toBe(3);
  });

  it('does not stretch widgets that are not side by side', () => {
    const row = [widget('a', 0, 0, 3), widget('b', 6, 0, 3)];
    const layouts = collapseHiddenWidgets(row, new Set(['b']));
    expect(layouts.get('a')).toEqual({ x: 0, y: 0, w: 3, h: 2 });
  });
});

describe('DashboardGrid hidden sensor cells', () => {
  it('collapses a cell whose sensor hides and restores it when the value returns', async () => {
    emptyIds.clear();
    setEmpty('b', true);
    const { container } = render(<DashboardGrid widgets={[...TILE_ROW, CHART]} ctx={{ vehicleId: 'v1', from: null, to: null }} />);

    const cell = (id: string) => container.querySelector(`[data-widget-id="${id}"]`) as HTMLElement;
    expect(cell('b')).toHaveAttribute('hidden');
    expect(cell('b')).toHaveAttribute('data-widget-collapsed', 'true');
    expect(cell('b')).toHaveClass('has-[[data-sensor-empty=hidden]]:hidden');
    expect(cell('a')).not.toHaveAttribute('hidden');
    expect(screen.getByTestId('tile-a')).toBeInTheDocument();

    await act(async () => {
      setEmpty('b', false);
      listeners.forEach((listener) => listener());
    });
    expect(cell('b')).not.toHaveAttribute('hidden');
    expect(cell('b')).not.toHaveAttribute('data-widget-collapsed');
    expect(screen.getByTestId('tile-b')).toBeInTheDocument();

    await act(async () => {
      setEmpty('a', true);
      listeners.forEach((listener) => listener());
    });
    expect(cell('a')).toHaveAttribute('hidden');
  });

  it('gives a chart cell with an empty row auto height on mobile and keeps the chart height otherwise', async () => {
    const original = window.innerWidth;
    Object.defineProperty(window, 'innerWidth', { configurable: true, value: 390 });
    const matchMedia = vi.spyOn(window, 'matchMedia').mockImplementation((query: string) => ({
      matches: query.includes('max-width: 639px'), media: query, onchange: null,
      addEventListener: vi.fn(), removeEventListener: vi.fn(), addListener: vi.fn(), removeListener: vi.fn(),
      dispatchEvent: vi.fn(),
    }) as unknown as MediaQueryList);
    try {
      emptyIds.clear();
      emptyCharts.clear();
      emptyCharts.add('empty-chart');
      const charts = [widget('empty-chart', 0, 0, 12, 10, 'chart'), widget('full-chart', 0, 10, 12, 10, 'chart')];
      const { container } = render(<DashboardGrid widgets={charts} ctx={{ vehicleId: 'v1', from: null, to: null }} />);
      const cell = (id: string) => container.querySelector(`[data-widget-id="${id}"]`) as HTMLElement;

      await waitFor(() => expect(cell('empty-chart')).toHaveAttribute('data-widget-empty-chart', 'true'));
      expect(cell('empty-chart').style.height).toBe('');
      expect(cell('full-chart')).not.toHaveAttribute('data-widget-empty-chart');
      expect(cell('full-chart').style.height).toBe('400px');

      await act(async () => {
        emptyCharts.delete('empty-chart');
        listeners.forEach((listener) => listener());
      });
      expect(cell('empty-chart')).not.toHaveAttribute('data-widget-empty-chart');
      expect(cell('empty-chart').style.height).toBe('400px');
    } finally {
      matchMedia.mockRestore();
      Object.defineProperty(window, 'innerWidth', { configurable: true, value: original });
    }
  });

  it('spans 2 desktop rows for an empty chart row and moves the next widget up', async () => {
    emptyIds.clear();
    emptyCharts.clear();
    emptyCharts.add('empty-chart');
    const { container } = render(
      <DashboardGrid
        widgets={[widget('empty-chart', 0, 0, 12, 10, 'chart'), widget('next', 0, 10, 12, 4, 'custom')]}
        ctx={{ vehicleId: 'v1', from: null, to: null }}
      />
    );
    const cell = (id: string) => container.querySelector(`[data-widget-id="${id}"]`) as HTMLElement;
    await waitFor(() => expect(cell('empty-chart').style.gridRow).toBe('1 / span 2'));
    expect(cell('next').style.gridRow).toBe('3 / span 4');
  });
});
