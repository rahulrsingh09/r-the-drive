import React from 'react';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Vehicle, VehicleStatus } from '@panda/types';

const fixture = vi.hoisted(() => ({
  vehicles: [] as Vehicle[],
  status: {} as VehicleStatus,
  statusRequests: vi.fn(),
  catalog: { data: [] as Array<{ id: string; name: string }>, isLoading: false, isError: false, isSuccess: true },
  efficiencyLoading: false,
  retry: vi.fn(),
}));
vi.mock('@panda/hooks', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@panda/hooks')>()),
  useAuth: () => ({ defaultVehicleId: 'default' }),
  useVehicles: () => ({ data: fixture.vehicles }),
  useCurrentVehicleStatus: (id: unknown) => {
    fixture.statusRequests(id);
    return { data: fixture.status };
  },
  useTripTags: () => ({ ...fixture.catalog, refetch: fixture.retry }),
  useEfficiencyByTag: () => ({ data: [], isLoading: fixture.efficiencyLoading }),
  useCreateTripTag: () => ({ mutateAsync: vi.fn(), isPending: false }),
}));
vi.mock('@tanstack/react-query', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@tanstack/react-query')>()),
  useQuery: () => ({ data: undefined, isLoading: false, isFetching: false }),
}));
vi.mock('@panda/ui/charts', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@panda/ui/charts')>()),
  EfficiencyPillBarChart: ({ loading, emptyTitle }: { loading: boolean; emptyTitle: string }) =>
    <div data-testid="tag-chart" data-loading={String(loading)}>{emptyTitle}</div>,
}));
import { getWidget } from '../../../../packages/dashboards/src/registry';
import type { WidgetCtx } from '../../../../packages/dashboards/src/registry';
import { CurrentVehicleStatePanel } from '../../../../packages/dashboards/src/widgets/custom/OverviewVehicleWidget';
import { DashboardChartRenderer } from '../../../../packages/dashboards/src/widgets/chart/DashboardChartWidget';
import { TripTagPicker } from '../../../../packages/dashboards/src/widgets/table/TripTagPicker';

const images = {
  overhead: { light: '/base-light.webp', dark: '/base-dark.webp' },
  all: [
    { placement: 'overhead', design: 'light', url: '/front-left-open-light.webp', size: 'large', resolution: '@3x' },
    { placement: 'overhead', design: 'dark', url: '/front-left-open-dark.webp', size: 'large', resolution: '@3x' },
  ],
};
const context: WidgetCtx = { vehicleId: 'selected', from: null, to: null };
function overview(vehicleId: string | null) {
  const Component = getWidget('custom', 'overview.vehicle')!.component;
  return <Component ctx={{ ...context, vehicleId }} instance={{
    id: 'overview', componentType: 'custom', definitionId: 'overview.vehicle',
    layout: { x: 0, y: 0, w: 12, h: 7 }, options: {},
  }} />;
}
beforeEach(() => {
  fixture.vehicles = [];
  fixture.status = { battery_level: 0, door_front_left_closed: false } as VehicleStatus;
  fixture.statusRequests.mockClear();
  fixture.catalog = { data: [], isLoading: false, isError: false, isSuccess: true };
  fixture.efficiencyLoading = false;
  fixture.retry.mockReset().mockResolvedValue({});
});
afterEach(cleanup);

describe('residual widget controls', () => {
  it.each([null, '', 'selected'])('selects the exact nullish vehicle identity %s', (vehicleId) => {
    render(overview(vehicleId));
    expect(fixture.statusRequests).toHaveBeenLastCalledWith(vehicleId ?? 'default');
    expect(screen.getByText('Vehicle overview')).toBeVisible();
    expect(screen.getByTestId('overview-charging-glyph')).toHaveAttribute('data-charging-state', 'disconnected');
  });

  it.each([false, true, undefined])('keeps explicit demo=%s ahead of the legacy identity prefix', async (is_demo) => {
    fixture.vehicles = [{ id: 'selected', display_name: 'Synthetic', model: 'R1T',
      rivian_vehicle_id: 'demo-synthetic', is_demo, images }] as Vehicle[];
    render(overview('selected'));
    const base = screen.getAllByAltText('Synthetic')[0]!;
    fireEvent.error(base);
    await waitFor(() => expect(base).toHaveAttribute('data-artwork-fallback', 'true'));
    expect(base).toHaveStyle({ left: is_demo === false ? '50%' : '53%',
      transform: `translate(-50%, -50%) rotate(90deg) scaleX(${509 / 446})` });
  });

  it('suppresses only the failing theme overlays and restores them after a primary source change', async () => {
    const view = render(<CurrentVehicleStatePanel status={fixture.status} images={images} vehicleModel="R1T" />);
    const base = screen.getAllByAltText('Rivian vehicle')[0]!;
    expect(view.container.querySelectorAll('img[alt=""]')).toHaveLength(2);
    fireEvent.error(base);
    await waitFor(() => expect(view.container.querySelectorAll('img[alt=""]')).toHaveLength(1));
    expect(view.container.querySelector('img[src="/front-left-open-dark.webp"]')).not.toBeNull();
    view.rerender(<CurrentVehicleStatePanel status={fixture.status}
      images={{ ...images, overhead: { ...images.overhead, light: '/recovered.webp' } }} vehicleModel="R1T" />);
    await waitFor(() => expect(view.container.querySelectorAll('img[alt=""]')).toHaveLength(2));
    expect(base).toHaveStyle({ left: '50%', transform: 'translate(-50%, -50%) rotate(90deg)' });
  });

  it.each([
    ['Disconnected', 'disconnected', 'Not connected'],
    ['Connected', 'connected', 'Connected, not charging'],
    ['Charging', 'charging', 'Charging'],
  ] as const)('keeps glyph state and accessible label for %s', (charger_state, state, label) => {
    render(<CurrentVehicleStatePanel status={{ ...fixture.status, charger_state }} />);
    const glyph = screen.getByTestId('overview-charging-glyph');
    expect(glyph).toHaveAttribute('data-charging-state', state);
    expect(glyph).toHaveAttribute('aria-label', label);
    expect(glyph.classList.contains('text-accent')).toBe(state === 'charging');
  });

  it('prioritizes catalog errors even with stale tags and simultaneous loading', () => {
    fixture.catalog = { data: [{ id: 'stale', name: 'Stale' }], isLoading: true, isError: true, isSuccess: false };
    render(<DashboardChartRenderer chartId="efficiency-tags" ctx={context} height={240} />);
    expect(screen.getByRole('alert')).toHaveTextContent('Couldn’t load shared tags');
    expect(screen.queryByTestId('tag-chart')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    expect(fixture.retry).toHaveBeenCalledTimes(1);
  });

  it.each([false, true])('keeps onboarding permission copy for manager=%s ahead of efficiency loading', (canManageTripTags) => {
    fixture.efficiencyLoading = true;
    render(<DashboardChartRenderer chartId="efficiency-tags" ctx={{ ...context, canManageTripTags }} height={240} />);
    expect(screen.getByText(canManageTripTags ? /Create one with the Tags filter above/ : /A vehicle manager can create tags/)).toBeVisible();
    expect(screen.queryByTestId('tag-chart')).toBeNull();
  });

  it('transitions from catalog loading to a loaded filtered cohort without clearing filters', () => {
    fixture.catalog.isLoading = true;
    const ctx = { ...context, tripTagFilter: { tagIds: [], tagMatch: 'any' as const, untagged: true } };
    const view = render(<DashboardChartRenderer chartId="efficiency-tags" ctx={ctx} height={240} />);
    expect(screen.getByTestId('tag-chart')).toHaveAttribute('data-loading', 'true');
    fixture.catalog.isLoading = false;
    fixture.catalog.data = [{ id: 'tag', name: 'Tag' }];
    view.rerender(<DashboardChartRenderer chartId="efficiency-tags" ctx={ctx} height={240} />);
    expect(screen.getByTestId('tag-chart')).toHaveAttribute('data-loading', 'false');
    expect(screen.getByTestId('tag-chart')).toHaveTextContent('No trips match these tag filters.');
  });

  it('focuses the desktop picker, ignores inside mousedown and releases outside dismissal on close', async () => {
    fixture.catalog.data = [{ id: 'tag', name: 'Tag' }];
    const view = render(<TripTagPicker vehicleId="selected" canManage={false} selectedIds={[]} onChange={vi.fn()} />);
    fireEvent.click(screen.getByRole('button', { name: /Filter tags/ }));
    const search = screen.getByRole('textbox');
    await waitFor(() => expect(search).toHaveFocus());
    fireEvent.mouseDown(search);
    expect(search).toBeVisible();
    fireEvent.mouseDown(document.body);
    expect(screen.queryByRole('textbox')).toBeNull();
    view.unmount();
    fireEvent.mouseDown(document.body);
  });
});
