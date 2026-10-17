import React from 'react';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { WidgetCtx, WidgetInstance } from '../../../../packages/dashboards/src/registry';

vi.mock('@panda/hooks', async (original) => ({
  ...(await original<typeof import('@panda/hooks')>()),
  useEffectiveCharts: () => ({ data: [], isSuccess: true }),
  useDashboardChartFavorites: () => ({ data: { chart_favorites: {} }, isSuccess: true }),
  useUpdateDashboardChartFavorite: () => ({ mutate: vi.fn() }),
  useSocHistory: () => ({ data: [], isLoading: false }),
  useRangeHistory: () => ({ data: [], isLoading: false }),
}));
vi.mock('@panda/ui/charts', async (original) => ({
  ...(await original<typeof import('@panda/ui/charts')>()),
  RichTimeSeriesChart: () => <div role="img" aria-label="Canonical chart fixture" />,
}));
import { DashboardChartWidget } from '../../../../packages/dashboards/src/widgets/chart/DashboardChartWidget';

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  window.history.replaceState({}, '', '/');
});

describe('widget display controls after layout editing removal', () => {
  it.each([393, 1280])('keeps saved geometry and display controls under ?edit=1 at %s px', (width) => {
    vi.stubGlobal('matchMedia', (query: string) => ({
      matches: query.includes('orientation') || (query.includes('max-width') && width < 640),
      media: query, addEventListener: vi.fn(), removeEventListener: vi.fn(),
      addListener: vi.fn(), removeListener: vi.fn(),
    }));
    window.history.replaceState({}, '', '/d/saved-layout?edit=1');
    const instance: WidgetInstance = {
      id: 'saved-chart', componentType: 'chart', definitionId: 'catalog', title: 'Saved chart',
      layout: Object.freeze({ x: 4, y: 7, w: 5, h: 9 }),
      options: {
        catalogMode: 'fixed', chartId: 'soc-history', chartIds: ['soc-history'], showPicker: false,
        chartSettings: { 'soc-history': { axes: { y: { mode: 'manual', min: 25, max: 95 } } } },
      },
    };
    const saved = structuredClone(instance);
    const { container } = render(<DashboardChartWidget instance={instance} ctx={{
      vehicleId: 'synthetic', from: null, to: null, dashboardSlug: 'saved-layout',
    }} />);
    expect(screen.getByRole('img', { name: 'Canonical chart fixture' })).toBeVisible();
    fireEvent.click(screen.getByRole('button', { name: 'Chart settings' }));
    expect(screen.getByText('Preview only while viewing.')).toBeVisible();
    expect(screen.queryByText(/dashboard edit mode/i)).toBeNull();
    expect(screen.queryByRole('button', { name: /Edit dashboard|Save dashboard|Discard/i })).toBeNull();
    expect(container.querySelector('.react-resizable-handle, .rgl-editor, [draggable="true"]')).toBeNull();
    expect(instance).toEqual(saved);
    fireEvent.keyDown(document, { key: 'Escape' });
    if (width < 640) {
      fireEvent.click(screen.getByRole('button', { name: 'Expand chart' }));
      expect(screen.getByRole('dialog', { name: /expanded chart/ })).toBeVisible();
      expect(screen.getByText('Rotate for a wider chart')).toBeVisible();
      fireEvent.click(screen.getByRole('button', { name: 'Close expanded chart' }));
      expect(screen.queryByRole('dialog', { name: /expanded chart/ })).toBeNull();
    }
  });

  it('saves only chart display settings through the existing optional callback', () => {
    const updateWidgetOptions = vi.fn<NonNullable<WidgetCtx['updateWidgetOptions']>>();
    render(<DashboardChartWidget instance={{
      id: 'saved-chart', componentType: 'chart', definitionId: 'catalog', title: 'Saved chart',
      layout: { x: 4, y: 7, w: 5, h: 9 },
      options: { catalogMode: 'fixed', chartId: 'soc-history', chartIds: ['soc-history'], showPicker: false },
    }} ctx={{ vehicleId: 'synthetic', from: null, to: null, updateWidgetOptions }} />);
    fireEvent.click(screen.getByRole('button', { name: 'Chart settings' }));
    fireEvent.change(screen.getByLabelText('Display filter'), { target: { value: '2' } });
    expect(updateWidgetOptions).toHaveBeenCalled();
    const [id, options] = updateWidgetOptions.mock.lastCall!;
    expect(id).toBe('saved-chart');
    expect(Object.keys(options)).toEqual(['chartSettings']);
    expect(options.chartSettings).toHaveProperty('soc-history.timeFilter', '1h');
  });
});
