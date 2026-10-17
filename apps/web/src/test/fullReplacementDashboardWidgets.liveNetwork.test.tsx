import React from 'react';
import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { WidgetCtx } from '../../../../packages/dashboards/src/registry';

const fixture = vi.hoisted(() => ({
  charging: true,
  loading: false,
  session: null as null | {
    power_kw: number | null;
    energy_kwh: number | null;
    soc_pct: number | null;
    time_remaining_min: number | null;
  },
  networks: [] as Array<{
    network_vendor: string | null;
    session_count: number;
    energy_kwh: number | null;
    cost_usd: number | null;
    free_sessions: number;
  }>,
}));
vi.mock('@panda/hooks', () => ({
  useCurrentVehicleStatus: () => ({
    data: {
      charger_state: fixture.charging ? 'Charging' : 'Disconnected',
      battery_level: 50,
      battery_limit: 80,
      range_miles: 200,
      time_to_end_of_charge_min: 90,
    },
  }),
  useLiveSession: () => ({ data: fixture.session, isLoading: fixture.loading }),
  useChargingSummary: () => ({
    data: { network_breakdown: fixture.networks },
    isLoading: fixture.loading,
  }),
}));
vi.mock('@iconify/react', () => ({ Icon: () => <span /> }));
import { getWidget } from '../../../../packages/dashboards/src/registry';
import '../../../../packages/dashboards/src/widgets/custom/LiveChargingWidget';
import '../../../../packages/dashboards/src/widgets/charging/NetworkBreakdownWidget';

function view(id: string, ctx: Partial<WidgetCtx> = {}) {
  const Component = getWidget('custom', id)!.component;
  return render(
    <Component
      instance={{
        id,
        componentType: 'custom',
        definitionId: id,
        options: {},
        layout: { x: 0, y: 0, w: 4, h: 4 },
      }}
      ctx={{ vehicleId: 'synthetic', from: null, to: null, ...ctx }}
    />
  );
}
beforeEach(() => {
  fixture.charging = true;
  fixture.loading = false;
  fixture.session = null;
  fixture.networks = [];
  localStorage.clear();
  vi.stubGlobal(
    'fetch',
    vi.fn(() => Promise.reject(new Error('Test forbids network access')))
  );
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe('live charging and network view contracts', () => {
  it('uses vehicle SoC and time while a charging session is missing', () => {
    view('overview.live.charging');
    expect(screen.getByRole('status')).toHaveTextContent('Live charging data unavailable');
    expect(screen.getByText('50%')).toBeVisible();
    expect(screen.getByText('1h 30m')).toBeVisible();
    expect(screen.getAllByText('—')).toHaveLength(2);
  });
  it('preserves zero-valued session readings instead of falling back to vehicle readings', () => {
    fixture.session = { power_kw: 0, energy_kwh: 0, soc_pct: 0, time_remaining_min: 0 };
    view('overview.live.charging');
    expect(screen.queryByRole('status')).toBeNull();
    for (const text of ['0.0 kW', '0.00 kWh', '0%', '—'])
      expect(screen.getByText(text)).toBeVisible();
  });
  it('uses vehicle data in the disconnected placeholder even when session data exists', () => {
    fixture.charging = false;
    fixture.session = { power_kw: 100, energy_kwh: 10, soc_pct: 80, time_remaining_min: 5 };
    view('overview.live.charging');
    expect(screen.getByText('Not plugged in')).toBeVisible();
    expect(screen.getByText('50%')).toBeVisible();
    expect(screen.getByText('200 mi')).toBeVisible();
    expect(screen.queryByText('100.0 kW')).toBeNull();
  });
  it('shows ordered network fractions, zero-cost omission, free sessions and unknown names', () => {
    fixture.networks = [
      {
        network_vendor: 'Home Charging',
        session_count: 3,
        energy_kwh: 75,
        cost_usd: 0,
        free_sessions: 2,
      },
      { network_vendor: null, session_count: 1, energy_kwh: 25, cost_usd: 5, free_sessions: 0 },
    ];
    const updateWidgetLayout = vi.fn();
    const { container } = view('charging.network_breakdown', { updateWidgetLayout });
    expect(screen.getByText('Home Charging')).toBeVisible();
    expect(screen.getByText('Unknown')).toBeVisible();
    expect(screen.getByText('2 free')).toBeVisible();
    expect(screen.getByText('$5.00')).toBeVisible();
    expect(screen.queryByText('$0.00')).toBeNull();
    expect(
      [...container.querySelectorAll<HTMLElement>('[style*="width:"]')].map((bar) =>
        Number.parseFloat(bar.style.width)
      )
    ).toEqual([75, 25]);
    expect(updateWidgetLayout).toHaveBeenCalled();
  });
  it('keeps loading skeletons distinct from the empty network view', () => {
    fixture.loading = true;
    const { container } = view('charging.network_breakdown');
    expect(container.querySelectorAll('.animate-pulse')).toHaveLength(3);
    expect(screen.queryByText('No charging sessions yet')).toBeNull();
    cleanup();
    fixture.loading = false;
    view('charging.network_breakdown');
    expect(screen.getByText('No charging sessions yet')).toBeVisible();
  });
});
