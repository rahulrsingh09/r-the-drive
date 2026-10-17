import React from 'react';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { DashboardGrid } from '../../../../packages/dashboards/src/DashboardGrid';
import { DashboardRenderer } from '../../../../packages/dashboards/src/DashboardRenderer';
import { WidgetChrome } from '../../../../packages/dashboards/src/WidgetChrome';
import { IconPicker } from '../../../../packages/dashboards/src/editor/IconPicker';
import type { DashboardConfig, WidgetInstance } from '../../../../packages/dashboards/src/schema';

const proxy = vi.hoisted(() => vi.fn());
vi.mock('@panda/hooks', async (original) => ({
  ...(await original<typeof import('@panda/hooks')>()),
  api: { proxyFetch: proxy },
  useAuth: () => ({ defaultVehicleId: null, isAdmin: true }),
  useCurrentVehicleStatus: () => ({ data: undefined }),
}));
vi.mock('../../../../packages/dashboards/src/dashboardData', () => ({
  DashboardDataProvider: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  collectDashboardDataRequirements: () => ({ latest: [], series: [] }),
}));
vi.mock('@iconify/react', () => ({ Icon: ({ icon }: { icon: string }) => <i data-icon={icon} />, _api: { setFetch: vi.fn() } }));
vi.mock('../../../../packages/dashboards/src/WidgetHost', () => ({ WidgetHost: ({ instance }: { instance: WidgetInstance }) => <span data-testid={`content-${instance.id}`} /> }));
const widget = (id: string, type: WidgetInstance['componentType'], definitionId = 'test'): WidgetInstance => ({ id, componentType: type, definitionId, layout: { x: 3, y: 2, w: 6, h: 20 } });
const ctx = { vehicleId: null, from: null, to: null };

afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); proxy.mockReset(); localStorage.clear(); });

describe('dashboard core interaction boundaries', () => {
  it('renders a frozen custom layout identically for view and legacy edit callers', () => {
    vi.stubGlobal('IntersectionObserver', undefined);
    const config: DashboardConfig = {
      schemaVersion: 2, id: 'custom', slug: 'personal-layout', name: 'Personal',
      isDefault: false, isLocked: false, ownerId: 'synthetic-owner',
      controls: { dateRange: true },
      widgets: [widget('custom-chart', 'chart'), widget('custom-sensor', 'sensor')],
    };
    const before = structuredClone(config);
    for (const instance of config.widgets) {
      Object.freeze(instance.layout);
      Object.freeze(instance);
    }
    Object.freeze(config.widgets);
    Object.freeze(config);
    const onConfigChange = vi.fn();
    const view = render(<DashboardRenderer config={config} ctx={ctx} />);
    const viewDom = view.container.innerHTML;
    view.unmount();
    window.history.replaceState({}, '', '/dashboard?edit=1');
    const edit = render(<DashboardRenderer
      config={config} ctx={ctx} mode="edit" onConfigChange={onConfigChange}
      editActions={<button onClick={() => onConfigChange(config)}>Save layout</button>}
    />);
    expect(edit.container.innerHTML).toBe(viewDom);
    const frame = edit.container.querySelector('[data-widget-id="custom-chart"]')!;
    expect(frame).toHaveStyle({ gridColumn: '4 / span 6', gridRow: '3 / span 20' });
    for (const event of ['pointerDown', 'pointerMove', 'pointerUp', 'dragStart', 'dragEnd', 'click'] as const) {
      fireEvent[event](frame);
    }
    fireEvent.keyDown(frame, { key: 'Delete' });
    expect(edit.container.querySelector('.react-grid-layout, .react-resizable-handle, .drag-handle, [data-widget-edit-control], [data-dashboard-preview-state]')).toBeNull();
    expect(screen.queryByRole('button')).toBeNull();
    expect(document.querySelector('[role="complementary"]')).toBeNull();
    expect(document.body).not.toHaveClass('rgl-drawer-open');
    expect(onConfigChange).not.toHaveBeenCalled();
    expect(config).toEqual(before);
    window.history.replaceState({}, '', '/');
  });
  it('ignores legacy chrome editor callbacks while preserving view children and frame props', () => {
    const onToggleEdit = vi.fn();
    const updateWidgetOptions = vi.fn();
    const instance = widget('standalone', 'sensor');
    const { container } = render(<WidgetChrome
      instance={instance} ctx={ctx} mode="edit" isEditing
      editor={{ movable: true, resizable: true, fixedSize: false, deprecated: false }}
      onToggleEdit={onToggleEdit} updateWidgetOptions={updateWidgetOptions}
      className="existing-class" aria-label="Saved widget"
    ><span>View child</span></WidgetChrome>);
    const frame = screen.getByLabelText('Saved widget');
    expect(frame).toHaveAttribute('data-widget-frame', 'view');
    expect(frame).toHaveClass('dashboard-widget-frame', 'existing-class');
    expect(screen.getByText('View child')).toBeInTheDocument();
    expect(container.querySelector('[data-editing], [data-widget-movable], [data-widget-resizable]')).toBeNull();
    fireEvent.click(frame);
    expect(onToggleEdit).not.toHaveBeenCalled();
    expect(updateWidgetOptions).not.toHaveBeenCalled();
  });
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
  it.each(['button', 'icon', 'label'] as const)('selects a recent icon through its %s and ignores grid whitespace', (target) => {
    localStorage.setItem('panda:icon-picker:recent', JSON.stringify(['lucide:car', 'mdi:home']));
    const onChange = vi.fn();
    render(<IconPicker value="mdi:home" onChange={onChange} />);
    fireEvent.click(screen.getByRole('button', { name: /Change/ }));
    const button = screen.getByTitle('lucide:car');
    const grid = button.parentElement!;
    expect(grid).toHaveClass('grid', 'grid-cols-4', 'gap-2');
    expect(screen.getByTitle('mdi:home')).toHaveClass('border-accent', 'bg-accent/15', 'text-accent');
    fireEvent.click(grid);
    expect(onChange).not.toHaveBeenCalled();
    fireEvent.click(target === 'button' ? button : button.querySelector(target === 'icon' ? 'i' : 'span')!);
    expect(onChange).toHaveBeenCalledExactlyOnceWith('lucide:car');
    expect(JSON.parse(localStorage.getItem('panda:icon-picker:recent')!)).toEqual(['lucide:car', 'mdi:home']);
    expect(proxy).not.toHaveBeenCalled();
  });
});
