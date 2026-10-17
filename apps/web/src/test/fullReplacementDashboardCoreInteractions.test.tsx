import React from 'react';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { DashboardGrid } from '../../../../packages/dashboards/src/DashboardGrid';
import { IconPicker } from '../../../../packages/dashboards/src/editor/IconPicker';
import type { WidgetInstance } from '../../../../packages/dashboards/src/schema';

const proxy = vi.hoisted(() => vi.fn());
vi.mock('@panda/hooks', async (original) => ({ ...(await original<typeof import('@panda/hooks')>()), api: { proxyFetch: proxy } }));
vi.mock('@iconify/react', () => ({ Icon: ({ icon }: { icon: string }) => <i data-icon={icon} />, _api: { setFetch: vi.fn() } }));
vi.mock('../../../../packages/dashboards/src/WidgetHost', () => ({ WidgetHost: ({ instance }: { instance: WidgetInstance }) => <span data-testid={`content-${instance.id}`} /> }));
const widget = (id: string, type: WidgetInstance['componentType'], definitionId = 'test'): WidgetInstance => ({ id, componentType: type, definitionId, layout: { x: 3, y: 2, w: 6, h: 20 } });
const ctx = { vehicleId: null, from: null, to: null };

afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); proxy.mockReset(); localStorage.clear(); });

describe('dashboard core interaction boundaries', () => {
  it('uses the exact 639/640 breakpoint and caps mobile chart height', () => {
    let update: ((event: { matches: boolean }) => void) | undefined;
    const remove = vi.fn();
    vi.stubGlobal('matchMedia', vi.fn(() => ({ matches: false, addEventListener: (_: string, fn: typeof update) => { update = fn; }, removeEventListener: remove })));
    vi.stubGlobal('IntersectionObserver', undefined);
    const { container, unmount } = render(<DashboardGrid widgets={[widget('chart', 'chart'), widget('sensor', 'sensor')]} ctx={ctx} />);
    const grid = container.querySelector('[data-dashboard-grid]') as HTMLElement;
    const chart = container.querySelector('[data-widget-id="chart"]') as HTMLElement;
    const sensor = container.querySelector('[data-widget-id="sensor"]') as HTMLElement;
    expect(window.matchMedia).toHaveBeenCalledWith('(max-width: 639px)');
    expect(grid.style.gridTemplateColumns).toBe('repeat(12, minmax(0, 1fr))');
    expect(chart.style.gridColumn).toBe('4 / span 6');
    act(() => update?.({ matches: true }));
    expect(grid.style.gridTemplateColumns).toBe('minmax(0, 1fr)');
    expect(chart.style.height).toBe('480px');
    expect(sensor.style.minHeight).toBe('480px');
    expect(chart.style.gridColumn).toBe('');
    unmount(); expect(remove).toHaveBeenCalled();
  });
  it('defers only expensive widgets and reveals them once at the 320px boundary', () => {
    let notify: IntersectionObserverCallback | undefined;
    const disconnect = vi.fn(); const observe = vi.fn();
    class Observer {
      constructor(callback: IntersectionObserverCallback, options: IntersectionObserverInit) { notify = callback; expect(options).toEqual({ rootMargin: '320px 0px' }); }
      observe = observe; disconnect = disconnect; unobserve = vi.fn();
    }
    vi.stubGlobal('IntersectionObserver', Observer);
    render(<DashboardGrid widgets={[widget('chart', 'chart'), widget('second', 'chart'), widget('light', 'custom')]} ctx={ctx} />);
    expect(screen.queryByTestId('content-chart')).toBeNull();
    expect(screen.getByTestId('content-light')).toBeInTheDocument();
    act(() => notify?.([{ isIntersecting: false, target: observe.mock.calls[0]![0] } as IntersectionObserverEntry], {} as IntersectionObserver));
    expect(screen.queryByTestId('content-chart')).toBeNull();
    act(() => notify?.([{ isIntersecting: true, target: observe.mock.calls[0]![0] } as IntersectionObserverEntry], {} as IntersectionObserver));
    expect(screen.getByTestId('content-chart')).toBeInTheDocument();
    expect(screen.queryByTestId('content-second')).toBeNull();
    expect(observe).toHaveBeenCalledTimes(2);
    act(() => notify?.([{ isIntersecting: true, target: observe.mock.calls[1]![0] } as IntersectionObserverEntry], {} as IntersectionObserver));
    expect(screen.getByTestId('content-second')).toBeInTheDocument();
    expect(disconnect).toHaveBeenCalled();
  });
  it('ignores an aborted search result, persists selection, and dismisses on Escape', async () => {
    let finishFirst: ((response: Response) => void) | undefined;
    proxy.mockImplementationOnce(() => new Promise<Response>(resolve => { finishFirst = resolve; }));
    proxy.mockResolvedValueOnce({ ok: true, json: async () => ({ icons: ['lucide:car'] }) });
    const onChange = vi.fn();
    render(<IconPicker value="battery" onChange={onChange} />);
    fireEvent.click(screen.getByRole('button', { name: /Change/ }));
    const search = screen.getByPlaceholderText('Search icons…');
    fireEvent.change(search, { target: { value: 'ba' } });
    await waitFor(() => expect(proxy).toHaveBeenCalledTimes(1));
    const firstSignal = proxy.mock.calls[0]![1].signal as AbortSignal;
    fireEvent.change(search, { target: { value: 'car' } });
    expect(firstSignal.aborted).toBe(true);
    await screen.findByTitle('lucide:car');
    await act(async () => finishFirst?.({ ok: true, json: async () => ({ icons: ['lucide:stale'] }) } as Response));
    expect(screen.queryByTitle('lucide:stale')).toBeNull();
    fireEvent.click(screen.getByTitle('lucide:car'));
    expect(onChange).toHaveBeenCalledWith('lucide:car');
    expect(JSON.parse(localStorage.getItem('panda:icon-picker:recent')!)).toEqual(['lucide:car']);
    fireEvent.click(screen.getByRole('button', { name: /Change/ }));
    fireEvent.keyDown(document, { key: 'Escape' });
    expect(screen.queryByPlaceholderText('Search icons…')).toBeNull();
    expect(proxy.mock.calls.every(([path]) => path.startsWith('/v1/external/iconify/search?'))).toBe(true);
  });
});
