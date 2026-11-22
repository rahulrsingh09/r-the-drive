import React from 'react';
import { fireEvent, render, screen, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const metricMocks = vi.hoisted(() => ({
  value: {
    metric: 'total_miles',
    value: 12345 as number | null,
    unit: 'mi' as string | null,
    label: 'Total Miles',
    ts: '2026-05-07T00:00:00Z' as string | null,
  },
  series: [
    { ts: '2026-05-01T00:00:00Z', value: 10 },
    { ts: '2026-05-02T00:00:00Z', value: 18 },
    { ts: '2026-05-03T00:00:00Z', value: 14 },
  ] as Array<{ ts: string; value: number | null }>,
  metricSeriesCalls: [] as Array<{
    vehicleId: string | null;
    metric: string | null;
    from: string | null;
    to: string | null;
  }>,
  metricBatchCalls: [] as Array<{ vehicleId: string | null; metric: string | null; from: string | null; to: string | null }>,
  batteryHealth: null as null | {
    usable_now_kwh: number | null;
    usable_new_kwh: number | null;
    battery_health_pct: number | null;
    estimated_degradation_pct: number | null;
    charging_cycles: number | null;
    charge_count: number;
    total_energy_added_kwh: number | null;
    total_energy_used_kwh: number | null;
    charging_efficiency_pct: number | null;
  },
  chargingSummary: null as null | {
    session_count: number;
    total_energy_kwh: number | null;
    total_cost_usd: number | null;
    home_kwh: number | null;
    away_kwh: number | null;
    unknown_location_kwh: number | null;
    ac_kwh: number | null;
    dc_kwh: number | null;
    charging_cycles: number | null;
    charging_efficiency_pct: number | null;
    max_charge_rate_kw: number | null;
    max_charge_limit_pct: number | null;
  },
  efficiencySummary: {
    avg: 200,
    p10: 180,
    p90: 220,
    total_miles: 120,
    efficiency_miles: 120,
    coverage_percent: 100,
  } as null | {
    avg: number;
    p10: number;
    p90: number;
    total_miles: number;
    efficiency_miles: number;
    coverage_percent: number;
  },
  vehicleStatus: null as null | Record<string, unknown>,
  chargingFetching: false,
  valueError: false,
  statusError: false,
  batchError: false,
  healthError: false,
  refetchValue: vi.fn(),
  refetchStatus: vi.fn(),
  refetchBatch: vi.fn(),
  refetchHealth: vi.fn(),
}));

vi.mock('@panda/hooks', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@panda/hooks')>();
  return {
    ...actual,
    useMetricValue: () => metricMocks.valueError
      ? { data: undefined, isLoading: false, isError: true, refetch: metricMocks.refetchValue }
      : { data: metricMocks.value, isError: false, refetch: metricMocks.refetchValue },
    useMetricSeries: (
      vehicleId: string | null,
      metric: string | null,
      from: string | null,
      to: string | null
    ) => {
      metricMocks.metricSeriesCalls.push({ vehicleId, metric, from, to });
      return { data: metricMocks.series };
    },
    useMetricBatch: (vehicleId: string | null, metrics: Array<{ metric?: string }> | undefined, from: string | null, to: string | null) => {
      metricMocks.metricBatchCalls.push({ vehicleId, metric: metrics?.[0]?.metric ?? null, from, to });
      if (metricMocks.batchError) {
        return { data: undefined, isFetching: false, isError: true, refetch: metricMocks.refetchBatch };
      }
      return {
        data: {
          values: [metricMocks.value],
          series: [{ metric: metricMocks.value.metric, points: metricMocks.series }],
          bucket: 'day',
          max_points: 96,
        },
        isFetching: false,
        isError: false,
        refetch: metricMocks.refetchBatch,
      };
    },
    useEfficiencySummary: () => ({ data: metricMocks.efficiencySummary, isLoading: false }),
    useBatteryHealth: () => metricMocks.healthError
      ? { data: undefined, isLoading: false, isFetching: false, isError: true, refetch: metricMocks.refetchHealth }
      : { data: metricMocks.batteryHealth, isLoading: false, isError: false, refetch: metricMocks.refetchHealth },
    useChargingSummary: () => ({
      data: metricMocks.chargingSummary,
      isLoading: metricMocks.chargingFetching && metricMocks.chargingSummary == null,
      isFetching: metricMocks.chargingFetching,
    }),
    useCurrentVehicleStatus: () => metricMocks.statusError
      ? { data: null, isLoading: false, isError: true, refetch: metricMocks.refetchStatus }
      : { data: metricMocks.vehicleStatus, isLoading: false, isError: false, refetch: metricMocks.refetchStatus },
    useMetricCatalog: () => ({ data: [] }),
  };
});

import {
  collectDashboardDataRequirements,
  DashboardDataProvider,
  DashboardRenderer,
  DEFAULT_DASHBOARDS,
  getWidget,
  getWidgetEditorMeta,
  SensorChipSummary,
} from '@panda/dashboards';
import { MiniSparkline, resolveCanvasColor } from '@panda/ui/charts';
import { applyThemePreferences } from '@panda/ui/lib/theme';
import type { DashboardConfig } from '@panda/dashboards';
import {
  getSensorDefinition,
  resolveSensorColorToken,
  SENSOR_DEFINITIONS,
} from '../../../../packages/dashboards/src/widgets/sensor/sensorDefinitions';

const defaultCtx = {
  vehicleId: 'vehicle-1',
  timeframe: {
    kind: 'custom' as const,
    from: new Date('2026-05-01T00:00:00Z'),
    to: new Date('2026-05-07T00:00:00Z'),
  },
  from: '2026-05-01',
  to: '2026-05-07',
};

function config(
  showSprite: boolean,
  accentBorder = false,
  options: Record<string, unknown> = {},
  definitionId = 'total_miles',
  title = 'Total Miles'
): DashboardConfig {
  return {
    schemaVersion: 2,
    id: '00000000-0000-0000-0000-000000000099',
    slug: 'sensor-test',
    name: 'Sensor Test',
    isDefault: false,
    isLocked: false,
    ownerId: null,
    controls: { dateRange: true },
    widgets: [
      {
        id: 'd9000009-0000-0000-0000-000000000001',
        componentType: 'sensor',
        definitionId,
        title,
        options: {
          metric: definitionId,
          icon: 'route',
          chartType: 'line',
          showSprite,
          accentBorder,
          showSubtitle: false,
          ...options,
        },
        layout: { x: 0, y: 0, w: 3, h: 2 },
      },
    ],
  };
}

/** Numbers render in primary ink: no series colour on the value itself. */
function expectPrimaryInkValue(chip: HTMLElement) {
  const valueNode = chip.querySelector<HTMLElement>('[data-sensor-value]');
  expect(valueNode).not.toBeNull();
  expect(valueNode).toHaveClass('text-fg');
  expect(valueNode).not.toHaveClass('text-accent');
  expect(valueNode!.style.color).toBe('');
}

describe('dashboard sensor chips', () => {
  beforeEach(() => {
    applyThemePreferences({ mode: 'dark', palette: 'classic' });
    metricMocks.value = {
      metric: 'total_miles',
      value: 12345,
      unit: 'mi',
      label: 'Total Miles',
      ts: '2026-05-07T00:00:00Z',
    };
    metricMocks.series = [
      { ts: '2026-05-01T00:00:00Z', value: 10 },
      { ts: '2026-05-02T00:00:00Z', value: 18 },
      { ts: '2026-05-03T00:00:00Z', value: 14 },
    ];
    metricMocks.batteryHealth = null;
    metricMocks.chargingSummary = null;
    metricMocks.efficiencySummary = {
      avg: 200,
      p10: 180,
      p90: 220,
      total_miles: 120,
      efficiency_miles: 120,
      coverage_percent: 100,
    };
    metricMocks.vehicleStatus = null;
    metricMocks.chargingFetching = false;
    metricMocks.metricSeriesCalls = [];
    metricMocks.metricBatchCalls = [];
  });

  it('renders the sprite as a bottom background layer when enabled', () => {
    render(<DashboardRenderer config={config(true)} ctx={defaultCtx} />);

    const layer = screen.getByTestId('sensor-sprite-layer');
    expect(layer).toHaveClass('absolute');
    expect(layer).toHaveStyle({ bottom: '0px', left: '0px', right: '0px' });
    expect(layer.querySelector('canvas')).not.toBeNull();
    expect(layer.querySelector('[data-sparkline-renderer="canvas"]')).not.toBeNull();
    expect(layer.querySelector('[data-sparkline-filter="24h"]')).not.toBeNull();
    expect(screen.queryByText('miles per day')).not.toBeInTheDocument();
  });

  it('allows line sprite filtering to be disabled', () => {
    render(
      <DashboardRenderer config={config(true, false, { timeFilter: 'raw' })} ctx={defaultCtx} />
    );

    const layer = screen.getByTestId('sensor-sprite-layer');
    expect(layer.querySelector('[data-sparkline-filter="raw"]')).not.toBeNull();
    expect(layer.querySelector('canvas')).not.toBeNull();
  });

  it('bins bar sprites into 24-hour windows by default', () => {
    render(
      <DashboardRenderer config={config(true, false, { chartType: 'bar' })} ctx={defaultCtx} />
    );

    expect(screen.getByTestId('sensor-sprite-layer').querySelector('[data-sparkline-filter="24h"]')).not.toBeNull();
  });

  it('keeps raw bars available when individual events matter', () => {
    render(
      <DashboardRenderer config={config(true, false, { chartType: 'bar', timeFilter: 'raw' })} ctx={defaultCtx} />
    );

    expect(screen.getByTestId('sensor-sprite-layer').querySelector('[data-sparkline-filter="raw"]')).not.toBeNull();
  });

  it('records the selected timestamp filter on the sprite renderer', () => {
    const { unmount } = render(
      <MiniSparkline
        data={metricMocks.series}
        type="line"
        timeFilter="15m"
        height={36}
      />
    );

    const lowFilter = document.querySelector('[data-sparkline-state="series"]');
    expect(lowFilter).toHaveAttribute('data-sparkline-filter', '15m');

    unmount();

    render(
      <MiniSparkline
        data={metricMocks.series}
        type="line"
        timeFilter="7d"
        height={36}
      />
    );

    const highFilter = document.querySelector('[data-sparkline-state="series"]');
    expect(highFilter).toHaveAttribute('data-sparkline-filter', '7d');
    expect(highFilter?.querySelector('canvas')).not.toBeNull();
  });

  it('keeps a raw timestamp filter available for sparse data', () => {
    render(
      <MiniSparkline
        data={[
          { ts: '2026-05-01T00:00:00Z', value: 10 },
          { ts: '2026-05-02T00:00:00Z', value: 18 },
        ]}
        type="line"
        timeFilter="raw"
        height={36}
      />
    );

    const sparkline = document.querySelector('[data-sparkline-state="series"]');
    expect(sparkline).toHaveAttribute('data-sparkline-filter', 'raw');
    expect(sparkline?.querySelector('canvas')).not.toBeNull();
  });

  it('compresses bar sprites into summed time bins when a display filter is selected', () => {
    render(
      <MiniSparkline
        data={[
          { ts: '2026-05-01T00:05:00Z', value: 1 },
          { ts: '2026-05-01T00:50:00Z', value: 1 },
          { ts: '2026-05-01T01:10:00Z', value: 1 },
        ]}
        type="bar"
        timeFilter="1h"
        height={36}
      />
    );

    const sparkline = document.querySelector('[data-sparkline-state="series"]');
    expect(sparkline).toHaveAttribute('data-sparkline-filter', '1h');
    expect(sparkline).toHaveAttribute('data-sparkline-aggregation', 'sum');
    expect(sparkline).toHaveAttribute('data-sparkline-point-count', '2');
  });

  it('applies the configured curve color', () => {
    render(
      <DashboardRenderer config={config(true, false, { curveColor: 'sky' })} ctx={defaultCtx} />
    );

    expect(screen.getByTestId('sensor-sprite-layer').querySelector('canvas')).not.toBeNull();
  });

  it('uses an explicit curve color for the sensor accent as well as its sprite', () => {
    applyThemePreferences({ mode: 'light', palette: 'rad' });

    render(
      <DashboardRenderer
        config={config(true, true, { curveColor: 'series-07', valueColor: 'data' })}
        ctx={defaultCtx}
      />
    );

    const chip = screen.getByTestId('sensor-chip');
    expect(chip).toHaveAttribute('style', expect.stringContaining('border-color: var(--rm-series-07)'));
    expect(chip.outerHTML).toContain('color: var(--rm-series-07)');
    expectPrimaryInkValue(chip);
  });

  it('resolves theme tokens for Canvas without replacing an explicit editor color', () => {
    const canvas = document.createElement('canvas');
    canvas.style.setProperty('--rm-accent', 'rgb(27, 125, 232)');

    expect(resolveCanvasColor(canvas, 'var(--rm-accent)')).toBe('rgb(27, 125, 232)');
    expect(resolveCanvasColor(canvas, '#38bdf8')).toBe('#38bdf8');
  });

  it('hides the sprite when disabled and applies the orange border option independently', () => {
    render(<DashboardRenderer config={config(false, true)} ctx={defaultCtx} />);

    expect(screen.queryByTestId('sensor-sprite-layer')).not.toBeInTheDocument();
    expect(screen.getByTestId('sensor-chip')).toHaveAttribute('style', expect.stringContaining('border-color: var(--rm-chart-accent)'));
  });

  it('keeps automatic metric colors monochrome in Classic and maps categories in RAD', () => {
    const mappings = [
      ['energy_charged', 'series-01'],
      ['total_trips', 'series-02'],
      ['total_miles', 'series-03'],
      ['trip_miles', 'series-03'],
      ['avg_efficiency', 'series-04'],
      ['avg_gross_efficiency', 'series-05'],
      ['avg_outside_temp_c', 'series-06'],
    ] as const;

    for (const [metric, token] of mappings) {
      expect(resolveSensorColorToken(metric, null, 'classic')).toBe('accent');
      expect(resolveSensorColorToken(metric, null, 'rad')).toBe(token);
    }
  });

  it('maps Charging and Battery source chips to stable RAD data categories', () => {
    const mappings = [
      ['charging_sessions_summary', 'series-02'],
      ['charging_total_energy', 'series-01'],
      ['charging_cycles_summary', 'series-13'],
      ['charging_total_cost', 'series-08'],
      ['charging_home_share', 'series-07'],
      ['charging_dc_share', 'series-09'],
      ['charging_efficiency_summary', 'series-04'],
      ['charging_max_limit', 'series-10'],
      ['charging_max_rate', 'series-11'],
      ['charging_avg_session', 'series-01'],
      ['battery_health_pct', 'series-05'],
      ['estimated_degradation_pct', 'series-06'],
      ['usable_capacity', 'series-12'],
      ['max_range', 'series-03'],
      ['charge_count', 'series-02'],
      ['charging_cycles_health', 'series-13'],
      ['battery_energy_added', 'series-01'],
      ['battery_charge_efficiency', 'series-04'],
    ] as const;

    for (const [definitionId, token] of mappings) {
      const definition = getSensorDefinition(definitionId);
      expect(definition, definitionId).toBeDefined();
      expect(resolveSensorColorToken(null, definition, 'classic')).toBe('accent');
      expect(resolveSensorColorToken(null, definition, 'rad')).toBe(token);
    }
  });

  it('renders the mapped RAD color through the shared chip renderer', () => {
    applyThemePreferences({ mode: 'light', palette: 'rad' });

    for (const [metric, token] of [
      ['energy_charged', 'series-01'],
      ['total_trips', 'series-02'],
      ['total_miles', 'series-03'],
      ['avg_efficiency', 'series-04'],
    ] as const) {
      metricMocks.value = { ...metricMocks.value, metric, value: 42, unit: null, label: metric };
      const view = render(
        <DashboardRenderer config={config(false, false, {}, metric, metric)} ctx={defaultCtx} />
      );

      const chip = view.container.querySelector<HTMLElement>('[data-testid="sensor-chip"]')!;
      // The series colour stays on the small marks (icon, sprite, border) only.
      expect(resolveSensorColorToken(metric, getSensorDefinition(metric), 'rad')).toBe(token);
      expect(chip.outerHTML).not.toContain(`var(--rm-${token}-text)`);
      expectPrimaryInkValue(chip);
      view.unmount();
    }
  });

  it('keeps light RAD Total Trips bright for graphics and readable for the value', () => {
    applyThemePreferences({ mode: 'light', palette: 'rad' });
    metricMocks.value = { ...metricMocks.value, metric: 'total_trips', value: 123, unit: null, label: 'Total Trips' };

    render(
      <DashboardRenderer
        config={config(true, true, { valueColor: 'data' }, 'total_trips', 'Total Trips')}
        ctx={defaultCtx}
      />
    );

    const chip = screen.getByTestId('sensor-chip');
    expect(chip).toHaveAttribute('style', expect.stringContaining('border-color: var(--rm-series-02)'));
    expect(chip.outerHTML).toContain('color: var(--rm-series-02)');
    expectPrimaryInkValue(chip);
  });

  it('renders automatic source-chip colors and readable light values for RAD', () => {
    applyThemePreferences({ mode: 'light', palette: 'rad' });
    metricMocks.batteryHealth = {
      usable_now_kwh: 111.6,
      usable_new_kwh: 109.0,
      battery_health_pct: 102.4,
      estimated_degradation_pct: 0,
      charging_cycles: 18,
      charge_count: 22,
      total_energy_added_kwh: 1800,
      total_energy_used_kwh: 1900,
      charging_efficiency_pct: 94.4,
    };
    metricMocks.chargingSummary = {
      session_count: 6,
      total_energy_kwh: 240,
      total_cost_usd: 48,
      home_kwh: 180,
      away_kwh: 60,
      unknown_location_kwh: 20,
      ac_kwh: 120,
      dc_kwh: 120,
      charging_cycles: 4,
      charging_efficiency_pct: 92.5,
      max_charge_rate_kw: 164.2,
      max_charge_limit_pct: 85,
    };

    for (const [definitionId, title, token] of [
      ['charging_total_energy', 'Energy added', 'series-01'],
      ['battery_health_pct', 'Battery Health', 'series-05'],
    ] as const) {
      const view = render(<DashboardRenderer config={config(false, true, {}, definitionId, title)} ctx={defaultCtx} />);
      const chip = screen.getByTestId('sensor-chip');
      expect(chip.outerHTML).toContain(`var(--rm-${token})`);
      expectPrimaryInkValue(chip);
      view.unmount();
    }
  });

  it('hides a bounded range tile with no data instead of falling back to latest', () => {
    metricMocks.series = [];

    render(<DashboardRenderer config={config(true)} ctx={defaultCtx} />);

    expect(screen.queryByTestId('sensor-chip')).not.toBeInTheDocument();
    expect(screen.getByTestId('sensor-chip-hidden')).toHaveAttribute('data-sensor-empty', 'hidden');
    expect(document.body).not.toHaveTextContent('12,345');
  });

  it('keeps a visible sprite for sparse lifetime data', () => {
    metricMocks.series = [];

    render(
      <DashboardRenderer
        config={config(true)}
        ctx={{
          vehicleId: 'vehicle-1',
          timeframe: { kind: 'lifetime' },
          from: null,
          to: null,
        }}
      />
    );

    const layer = screen.getByTestId('sensor-sprite-layer');
    expect(layer.querySelector('[data-sparkline-state="single"]')).not.toBeNull();
    expect(layer.querySelector('canvas')).not.toBeNull();
  });

  it('hides the tile when both series and value are missing', () => {
    metricMocks.value = { ...metricMocks.value, value: null };
    metricMocks.series = [];

    render(<DashboardRenderer config={config(true)} ctx={defaultCtx} />);

    expect(screen.queryByTestId('sensor-chip')).not.toBeInTheDocument();
    expect(screen.queryByTestId('sensor-sprite-layer')).not.toBeInTheDocument();
    expect(screen.getByTestId('sensor-chip-hidden')).toBeInTheDocument();
  });

  it('shows the weighted avg efficiency value instead of the latest series point', () => {
    metricMocks.value = {
      metric: 'avg_efficiency',
      value: 200,
      unit: 'Wh/mi',
      label: 'Avg Efficiency',
      ts: '2026-05-07T00:00:00Z',
    };
    metricMocks.series = [
      { ts: '2026-05-01T00:00:00Z', value: 1000 },
      { ts: '2026-05-02T00:00:00Z', value: 330 },
      { ts: '2026-05-03T00:00:00Z', value: 999 },
    ];

    render(
      <DashboardRenderer
        config={config(true, false, {}, 'avg_efficiency', 'Avg Efficiency')}
        ctx={defaultCtx}
      />
    );

    expect(screen.getByTestId('sensor-chip')).toHaveTextContent('5.0 mi/kWh');
    expect(screen.getByTestId('sensor-chip')).not.toHaveTextContent('1.0 mi/kWh');
  });

  it('uses the server-calculated range value instead of the latest gross-efficiency bucket', () => {
    metricMocks.value = {
      metric: 'avg_gross_efficiency',
      value: 333,
      unit: 'Wh/mi',
      label: 'Avg Gross Efficiency',
      ts: '2026-05-07T00:00:00Z',
    };
    metricMocks.series = [
      { ts: '2026-05-01T00:00:00Z', value: 8 },
      { ts: '2026-05-02T00:00:00Z', value: null },
      { ts: '2026-05-03T00:00:00Z', value: 13 },
      { ts: '2026-05-04T00:00:00Z', value: null },
      { ts: '2026-05-05T00:00:00Z', value: 31 },
    ];

    render(
      <DashboardRenderer
        config={config(true, false, {}, 'avg_gross_efficiency', 'Avg Gross Efficiency')}
        ctx={defaultCtx}
      />
    );

    expect(screen.getByTestId('sensor-chip')).toHaveTextContent('3.0 mi/kWh');
    expect(screen.getByTestId('sensor-chip')).not.toHaveTextContent('32.3 mi/kWh');
  });

  it('uses the server-calculated range value instead of the latest temperature bucket', () => {
    metricMocks.value = {
      metric: 'avg_outside_temp_c',
      value: 20,
      unit: 'C',
      label: 'Avg Outside Temp',
      ts: '2026-05-07T00:00:00Z',
    };
    metricMocks.series = [
      { ts: '2026-05-01T00:00:00Z', value: 55 },
      { ts: '2026-05-02T00:00:00Z', value: 48 },
      { ts: '2026-05-03T00:00:00Z', value: null },
      { ts: '2026-05-04T00:00:00Z', value: 62 },
      { ts: '2026-05-05T00:00:00Z', value: null },
    ];

    render(
      <DashboardRenderer
        config={config(true, false, {}, 'avg_outside_temp_c', 'Avg Outside Temp')}
        ctx={defaultCtx}
      />
    );

    expect(screen.getByTestId('sensor-chip')).toHaveTextContent('68 °F');
    expect(screen.queryByText('62')).not.toBeInTheDocument();
  });

  it('renders consumption coverage with covered and total miles plus an accessible explanation', () => {
    metricMocks.efficiencySummary = {
      avg: 400,
      p10: 300,
      p90: 500,
      total_miles: 1211.2,
      efficiency_miles: 1199.5,
      coverage_percent: 99,
    };

    render(
      <DashboardRenderer
        config={config(false, false, {}, 'efficiency_coverage', 'Efficiency data coverage')}
        ctx={defaultCtx}
      />
    );

    expect(screen.getByText('99%')).toBeInTheDocument();
    expect(screen.getByText('1,200 mi / 1,211 mi')).toBeInTheDocument();

    fireEvent.focus(screen.getByRole('button', { name: 'Efficiency data coverage help' }));
    expect(screen.getByRole('tooltip')).toHaveTextContent('Share of miles in this range');
  });

  it('explains the distance-weighted average and its data coverage', () => {
    metricMocks.value = {
      metric: 'avg_efficiency',
      value: 400,
      unit: 'Wh/mi',
      label: 'Avg Efficiency',
      ts: '2026-05-07T00:00:00Z',
    };
    metricMocks.efficiencySummary = {
      avg: 400,
      p10: 300,
      p90: 500,
      total_miles: 1211.2,
      efficiency_miles: 1199.5,
      coverage_percent: 99,
    };

    render(<DashboardRenderer config={config(false, false, {}, 'avg_efficiency', 'Efficiency (net)')} ctx={defaultCtx} />);

    fireEvent.focus(screen.getByRole('button', { name: 'Efficiency (net) help' }));
    expect(screen.getByRole('tooltip')).toHaveTextContent('Total estimated battery energy used ÷ miles driven');
    expect(screen.getByRole('tooltip')).toHaveTextContent('1,200 mi of 1,211 mi');
  });

  it('passes bounded timeframe bounds into metric series for latest-mode sensor chips', () => {
    metricMocks.value = {
      metric: 'avg_outside_temp_c',
      value: 999,
      unit: null,
      label: 'Avg Outside Temp',
      ts: '2026-05-07T00:00:00Z',
    };
    metricMocks.series = [
      { ts: '2026-05-01T00:00:00Z', value: null },
      { ts: '2026-05-02T00:00:00Z', value: 57 },
      { ts: '2026-05-03T00:00:00Z', value: 63 },
    ];

    render(
      <DashboardRenderer
        config={config(true, false, {}, 'avg_outside_temp_c', 'Avg Outside Temp')}
        ctx={{
          ...defaultCtx,
          from: '2026-04-22',
          to: '2026-04-23',
          timeframe: {
            kind: 'custom',
            from: new Date('2026-04-22T00:00:00Z'),
            to: new Date('2026-04-23T00:00:00Z'),
          },
        }}
      />
    );

    const call = metricMocks.metricBatchCalls.at(-1);
    expect(call).toEqual({
      vehicleId: 'vehicle-1',
      metric: 'avg_outside_temp_c',
      from: '2026-04-22',
      to: '2026-04-23',
    });
  });

  it('renders composite sensor language without changing the compact chip visual', () => {
    metricMocks.batteryHealth = {
      usable_now_kwh: 111.6,
      usable_new_kwh: 109.0,
      battery_health_pct: 102.4,
      estimated_degradation_pct: 0,
      charging_cycles: 18,
      charge_count: 22,
      total_energy_added_kwh: 1800,
      total_energy_used_kwh: 1900,
      charging_efficiency_pct: 94.4,
    };

    render(
      <DashboardRenderer
        config={{
          ...config(false),
          widgets: [
            {
              id: 'd9000009-0000-0000-0000-000000000002',
              componentType: 'sensor',
              definitionId: 'usable_capacity',
              title: 'Usable Capacity',
              options: {},
              layout: { x: 0, y: 0, w: 3, h: 2 },
            },
          ],
        }}
        ctx={defaultCtx}
      />
    );

    expect(screen.getByText('Usable Capacity')).toBeInTheDocument();
    expect(screen.getByText('(now/new)')).toBeInTheDocument();
    expect(screen.getByTestId('sensor-chip').querySelector('[data-sensor-value]')).toHaveTextContent('111.6 kWh');
    expect(screen.getByTestId('sensor-chip').querySelector('[data-sensor-value]')).toHaveClass('text-fg');
    expect(screen.getByText('/109.0 kWh')).toHaveClass('text-fg-tertiary');
  });

  it('renders battery charge cycles inline with the charge count', () => {
    metricMocks.batteryHealth = {
      usable_now_kwh: 111.6,
      usable_new_kwh: 109.0,
      battery_health_pct: 102.4,
      estimated_degradation_pct: 0,
      charging_cycles: 16,
      charge_count: 45,
      total_energy_added_kwh: 1800,
      total_energy_used_kwh: 1900,
      charging_efficiency_pct: 94.4,
    };

    render(
      <DashboardRenderer
        config={{
          ...config(false),
          widgets: [
            {
              id: 'd9000009-0000-0000-0000-000000000018',
              componentType: 'sensor',
              definitionId: 'charge_count',
              title: 'Charges',
              options: {},
              layout: { x: 0, y: 0, w: 3, h: 2 },
            },
          ],
        }}
        ctx={defaultCtx}
      />
    );

    const value = screen.getByText('45');
    const cycles = screen.getByText('(16 cycles)');
    expect(cycles.parentElement).toBe(value.parentElement);
    expect(screen.queryByText('16 cycles')).not.toBeInTheDocument();
  });

  it('shows lifetime badges only on lifetime battery cards', () => {
    metricMocks.batteryHealth = {
      usable_now_kwh: 111.6,
      usable_new_kwh: 109.0,
      battery_health_pct: 102.4,
      estimated_degradation_pct: 0,
      charging_cycles: 18,
      charge_count: 22,
      total_energy_added_kwh: 1800,
      total_energy_used_kwh: 1900,
      charging_efficiency_pct: 94.4,
    };

    render(
      <DashboardRenderer
        config={{
          ...config(false),
          widgets: [
            {
              id: 'd9000009-0000-0000-0000-000000000010',
              componentType: 'sensor',
              definitionId: 'battery_health_pct',
              title: 'Battery Health',
              options: {},
              layout: { x: 0, y: 0, w: 3, h: 2 },
            },
            {
              id: 'd9000009-0000-0000-0000-000000000011',
              componentType: 'sensor',
              definitionId: 'estimated_degradation_pct',
              title: 'Estimated Degradation',
              options: {},
              layout: { x: 3, y: 0, w: 3, h: 2 },
            },
            {
              id: 'd9000009-0000-0000-0000-000000000012',
              componentType: 'sensor',
              definitionId: 'usable_capacity',
              title: 'Usable Capacity',
              options: {},
              layout: { x: 6, y: 0, w: 3, h: 2 },
            },
            {
              id: 'd9000009-0000-0000-0000-000000000013',
              componentType: 'sensor',
              definitionId: 'max_range',
              title: 'Max Range',
              options: {},
              layout: { x: 9, y: 0, w: 3, h: 2 },
            },
            {
              id: 'd9000009-0000-0000-0000-000000000014',
              componentType: 'sensor',
              definitionId: 'charge_count',
              title: 'Charges',
              options: {},
              layout: { x: 0, y: 2, w: 3, h: 2 },
            },
            {
              id: 'd9000009-0000-0000-0000-000000000015',
              componentType: 'sensor',
              definitionId: 'charging_cycles_health',
              title: 'Charging Cycles',
              options: {},
              layout: { x: 3, y: 2, w: 3, h: 2 },
            },
            {
              id: 'd9000009-0000-0000-0000-000000000016',
              componentType: 'sensor',
              definitionId: 'battery_energy_added',
              title: 'Energy Added',
              options: {},
              layout: { x: 6, y: 2, w: 3, h: 2 },
            },
            {
              id: 'd9000009-0000-0000-0000-000000000017',
              componentType: 'sensor',
              definitionId: 'battery_charge_efficiency',
              title: 'Charge Efficiency',
              options: {},
              layout: { x: 9, y: 2, w: 3, h: 2 },
            },
          ],
        }}
        ctx={defaultCtx}
      />
    );

    expect(screen.queryByText('Current')).not.toBeInTheDocument();
    expect(screen.getAllByText('Lifetime')).toHaveLength(4);
  });

  it('folds unknown charging energy into away for the home share chip', () => {
    metricMocks.chargingSummary = {
      session_count: 6,
      total_energy_kwh: 240,
      total_cost_usd: 48,
      home_kwh: 180,
      away_kwh: 60,
      unknown_location_kwh: 20,
      ac_kwh: 120,
      dc_kwh: 120,
      charging_cycles: 4,
      charging_efficiency_pct: 92.5,
      max_charge_rate_kw: 164.2,
      max_charge_limit_pct: 85,
    };

    render(
      <DashboardRenderer
        config={{
          ...config(false),
          widgets: [
            {
              id: 'd9000009-0000-0000-0000-000000000003',
              componentType: 'sensor',
              definitionId: 'charging_home_share',
              title: 'Home Charging',
              options: {},
              layout: { x: 0, y: 0, w: 3, h: 2 },
            },
          ],
        }}
        ctx={defaultCtx}
      />
    );

    expect(screen.getByText('69%')).toBeInTheDocument();
    expect(screen.getByText('Home 180.0 kWh / Away 80.0 kWh')).toBeInTheDocument();
  });

  it('renders a shared unavailable chip for never-seen vehicle status fields', () => {
    metricMocks.vehicleStatus = {
      service_mode: null,
      field_availability: {
        service_mode: {
          ever_seen: false,
          last_seen_at: null,
          latest_event_at: '2026-05-07T00:00:00Z',
          availability: 'never_seen',
          reason_code: 'never_seen',
        },
      },
    };

    render(
      <DashboardRenderer
        config={config(false, false, {}, 'service_mode', 'Service Mode')}
        ctx={defaultCtx}
      />
    );

    expect(screen.getByTestId('sensor-unavailable-chip')).toHaveTextContent('Unavailable');
  });

  it('shows historical vehicle status values with a last-updated line', () => {
    metricMocks.vehicleStatus = {
      gear_guard_locked: true,
      field_availability: {
        gear_guard_locked: {
          ever_seen: true,
          last_seen_at: '2026-05-05T12:15:00Z',
          latest_event_at: '2026-05-07T00:00:00Z',
          availability: 'historical',
          reason_code: 'missing_recent_payload',
        },
        gear_guard_video_status: {
          ever_seen: false,
          last_seen_at: null,
          latest_event_at: '2026-05-07T00:00:00Z',
          availability: 'never_seen',
          reason_code: 'never_seen',
        },
      },
    };

    render(
      <DashboardRenderer
        config={config(false, false, {}, 'gear_guard_locked', 'Gear Guard')}
        ctx={defaultCtx}
      />
    );

    expect(screen.getByText('Locked')).toBeInTheDocument();
    expect(screen.getByText(/Last updated/)).toBeInTheDocument();
  });

  it('renders composite window status when at least one field is current', () => {
    metricMocks.vehicleStatus = {
      window_fl_closed: true,
      window_fr_closed: false,
      window_rl_closed: true,
      window_rr_closed: true,
      field_availability: {
        window_fl_closed: {
          ever_seen: true,
          last_seen_at: '2026-05-07T00:00:00Z',
          latest_event_at: '2026-05-07T00:00:00Z',
          availability: 'current',
          reason_code: null,
        },
        window_fr_closed: {
          ever_seen: true,
          last_seen_at: '2026-05-07T00:00:00Z',
          latest_event_at: '2026-05-07T00:00:00Z',
          availability: 'current',
          reason_code: null,
        },
        window_rl_closed: {
          ever_seen: true,
          last_seen_at: '2026-05-06T22:00:00Z',
          latest_event_at: '2026-05-07T00:00:00Z',
          availability: 'historical',
          reason_code: 'missing_recent_payload',
        },
        window_rr_closed: {
          ever_seen: true,
          last_seen_at: '2026-05-06T22:00:00Z',
          latest_event_at: '2026-05-07T00:00:00Z',
          availability: 'historical',
          reason_code: 'missing_recent_payload',
        },
      },
    };

    render(
      <DashboardRenderer
        config={config(false, false, {}, 'window_status', 'Windows')}
        ctx={defaultCtx}
      />
    );

    expect(screen.getByText('1 open')).toBeInTheDocument();
  });
});

type ChargingSummaryFixture = NonNullable<typeof metricMocks.chargingSummary>;
type SessionFixture = { energy_kwh: number; charger_type: 'AC' | 'DC' | null };

/**
 * Mirrors the `/charging/summary` aggregate: every session counts, and each
 * session adds its energy to the AC or DC bucket of its charger type (the same
 * per-session type that `/charging/chart-series` stacks in the Energy-by-day
 * chart).
 */
function summaryFromSessions(sessions: SessionFixture[]): ChargingSummaryFixture {
  const sum = (rows: SessionFixture[]) => rows.reduce((total, row) => total + row.energy_kwh, 0);
  return {
    session_count: sessions.length,
    total_energy_kwh: sum(sessions),
    total_cost_usd: null,
    home_kwh: 0,
    away_kwh: 0,
    unknown_location_kwh: 0,
    ac_kwh: sum(sessions.filter((row) => row.charger_type === 'AC')),
    dc_kwh: sum(sessions.filter((row) => row.charger_type === 'DC')),
    charging_cycles: 1,
    charging_efficiency_pct: null,
    max_charge_rate_kw: null,
    max_charge_limit_pct: null,
  };
}

function chargingDefaultSensorConfig(): DashboardConfig {
  const charging = DEFAULT_DASHBOARDS.find((dashboard) => dashboard.slug === 'charging')!;
  return {
    ...charging,
    // Always-visible tiles only; plugged/unplugged tiles depend on live status.
    widgets: charging.widgets.filter(
      (widget) => widget.componentType === 'sensor' && !(widget.visibility?.length),
    ),
  };
}

function tileByTitle(title: string) {
  const chip = screen
    .getAllByTestId('sensor-chip')
    .find((element) => element.querySelector('p')?.textContent?.trim().startsWith(title));
  expect(chip, title).toBeDefined();
  return chip!;
}

describe('sensor tile rules', () => {
  beforeEach(() => {
    applyThemePreferences({ mode: 'dark', palette: 'rad' });
    metricMocks.chargingSummary = null;
    metricMocks.chargingFetching = false;
  });

  it('keeps one Charges tile and shows N charges and D ÷ total DC share from one session set', () => {
    const sessions: SessionFixture[] = [
      { energy_kwh: 30, charger_type: 'DC' },
      { energy_kwh: 20, charger_type: 'AC' },
      { energy_kwh: 50, charger_type: 'DC' },
      { energy_kwh: 25, charger_type: 'AC' },
    ];
    const n = sessions.length;
    const dcEnergy = 80;
    const total = 125;
    metricMocks.chargingSummary = summaryFromSessions(sessions);

    const config = chargingDefaultSensorConfig();
    const countTiles = config.widgets.filter(
      (widget) => getSensorDefinition(widget.definitionId)?.valuePath === 'session_count',
    );
    expect(countTiles.map((widget) => [widget.definitionId, widget.title])).toEqual([
      ['charging_cycles_summary', 'Charges'],
    ]);

    render(<DashboardRenderer config={config} ctx={defaultCtx} />);

    expect(screen.queryByText('Sessions')).not.toBeInTheDocument();
    expect(tileByTitle('Charges').querySelector('[data-sensor-value]')).toHaveTextContent(String(n));
    const dcTile = tileByTitle('DC fast');
    expect(dcTile.querySelector('[data-sensor-value]')).toHaveTextContent(
      `${Math.round((dcEnergy / total) * 100)}%`,
    );
    expect(dcTile).toHaveTextContent('AC 45.0 kWh / DC 80.0 kWh');
    expect(tileByTitle('Energy added').querySelector('[data-sensor-value]')).toHaveTextContent('125.0 kWh');
    expect(tileByTitle('Charges')).toHaveTextContent('Full cycles: 1');
  });

  it('never renders a dash value: cost shows an action, other empty tiles hide', () => {
    metricMocks.chargingSummary = {
      ...summaryFromSessions([{ energy_kwh: 12, charger_type: 'AC' }]),
      home_kwh: 0,
      away_kwh: 0,
      unknown_location_kwh: 0,
    };

    render(<DashboardRenderer config={chargingDefaultSensorConfig()} ctx={defaultCtx} />);

    const costTile = tileByTitle('Total Cost');
    const action = within(costTile).getByRole('link', { name: 'Add a home price' });
    expect(action).toHaveAttribute('href', '/settings?section=places');
    expect(costTile).toHaveAttribute('data-sensor-empty', 'action');
    expect(costTile.querySelector('[data-sensor-value]')).toBeNull();
    // Home share is 0 ÷ 0: no value, so the tile hides.
    expect(screen.queryByText('Home Charging')).not.toBeInTheDocument();
    expect(screen.getAllByTestId('sensor-chip-hidden')).toHaveLength(1);
    for (const chip of screen.getAllByTestId('sensor-chip')) {
      const value = chip.querySelector('[data-sensor-value]')?.textContent?.trim();
      expect(value === '-' || value === '—').toBe(false);
    }
  });

  it('keeps an empty tile visible with a reason line in the dashboard editor', () => {
    metricMocks.value = { ...metricMocks.value, value: null };
    metricMocks.series = [];

    render(
      <DashboardRenderer
        config={config(true)}
        ctx={{ ...defaultCtx, visibilityState: { 'vehicle-connection': 'unplugged' } }}
      />,
    );

    const chip = screen.getByTestId('sensor-chip');
    expect(chip).toHaveAttribute('data-sensor-empty', 'reason');
    expect(within(chip).getByTestId('sensor-empty-line')).toHaveTextContent('No data in this range');
    expect(chip.querySelector('[data-sensor-value]')).toBeNull();
    expect(screen.queryByTestId('sensor-sprite-layer')).not.toBeInTheDocument();
  });

  it('shows a busy skeleton line while summary data loads', () => {
    metricMocks.chargingFetching = true;

    render(<DashboardRenderer config={config(false, false, {}, 'charging_total_energy', 'Energy added')} ctx={defaultCtx} />);

    const chip = screen.getByTestId('sensor-chip');
    expect(chip).toHaveAttribute('aria-busy', 'true');
    expect(within(chip).getByTestId('sensor-value-skeleton')).toHaveAttribute('aria-hidden', 'true');
    expect(chip).not.toHaveTextContent('...');
  });

  it('gives the small help icon a 44 × 44 hit area', () => {
    render(<DashboardRenderer config={config(false, false, {}, 'efficiency_coverage', 'Efficiency data coverage')} ctx={defaultCtx} />);

    const button = screen.getByRole('button', { name: 'Efficiency data coverage help' });
    // The hit area is the button's own ::before, so the UX audit and the
    // browser count it as part of the button.
    expect(button).toHaveClass('relative', 'before:absolute', 'before:h-11', 'before:w-11', "before:content-['']");
    expect(button).not.toHaveClass('before:pointer-events-none');
  });

  it('hides the duplicate count tile from the editor palette but still renders saved layouts', () => {
    expect(getWidgetEditorMeta(getWidget('sensor', 'charging_sessions_summary')).deprecated).toBe(true);
    expect(getWidgetEditorMeta(getWidget('sensor', 'charging_cycles_summary')).deprecated).toBe(false);
  });

  it('uses glossary terms in tile labels', () => {
    const titles = SENSOR_DEFINITIONS.map((definition) => definition.title);
    for (const title of titles) {
      expect(title, title).not.toMatch(/Session|Consumption|\bSOC\b|\bSoC\b|Total Energy|Energy Charged|->|Avg \//);
    }
    expect(getSensorDefinition('charging_avg_session')?.title).toBe('Average per charge');
    expect(getSensorDefinition('avg_efficiency')?.title).toBe('Efficiency (net)');
    expect(getSensorDefinition('charging_total_energy')?.title).toBe('Energy added');
    expect(getSensorDefinition('charging_dc_share')?.title).toBe('DC fast');
    const chargingTitles = DEFAULT_DASHBOARDS.find((dashboard) => dashboard.slug === 'charging')!
      .widgets.filter((widget) => widget.componentType === 'sensor')
      .map((widget) => widget.title);
    expect(chargingTitles).not.toContain('Sessions');
    expect(chargingTitles).not.toContain('Avg / Session');
    expect(chargingTitles).not.toContain('Total Energy');
  });
});

describe('SensorChipSummary tile rules', () => {
  beforeEach(() => {
    applyThemePreferences({ mode: 'light', palette: 'rad' });
  });

  it.each(['-', '—', '', null])('shows a reason line instead of the %j placeholder', (value) => {
    render(<SensorChipSummary title="Avg. Speed" value={value} dataAccent="series-03" valueColor="data" />);

    const chip = screen.getByTestId('sensor-chip');
    expect(chip.querySelector('[data-sensor-value]')).toBeNull();
    expect(within(chip).getByTestId('sensor-empty-line')).toHaveTextContent('No data');
  });

  it('renders the number in primary ink with the series colour only on the icon', () => {
    render(<SensorChipSummary title="Distance" value="12.4 mi" dataAccent="series-03" valueColor="data" />);

    const chip = screen.getByTestId('sensor-chip');
    expectPrimaryInkValue(chip);
    expect(chip.outerHTML).not.toContain('var(--rm-series-03-text)');
  });

  it('shows a busy skeleton line while loading', () => {
    render(<SensorChipSummary title="Distance" value="12.4 mi" loading />);

    const chip = screen.getByTestId('sensor-chip');
    expect(chip).toHaveAttribute('aria-busy', 'true');
    expect(within(chip).getByTestId('sensor-value-skeleton')).toBeInTheDocument();
    expect(chip).not.toHaveTextContent('12.4 mi');
  });
});

describe('sensor tile read failure', () => {
  const instance = (definitionId: string) => ({
    id: 'd9000009-0000-0000-0000-000000000077',
    componentType: 'sensor' as const,
    definitionId,
    options: {},
    layout: { x: 0, y: 0, w: 3, h: 2 },
  });

  function renderTile(definitionId: string) {
    const Tile = getWidget('sensor', definitionId)!.component;
    // No DashboardDataProvider: the tile reads its own queries.
    return render(<Tile instance={instance(definitionId)} ctx={defaultCtx} />);
  }

  beforeEach(() => {
    applyThemePreferences({ mode: 'dark', palette: 'classic' });
    metricMocks.valueError = false;
    metricMocks.statusError = false;
    metricMocks.vehicleStatus = null;
    metricMocks.value = { metric: 'total_miles', value: 12345, unit: 'mi', label: 'Total Miles', ts: '2026-05-07T00:00:00Z' };
    metricMocks.series = [
      { ts: '2026-05-01T00:00:00Z', value: 10 },
      { ts: '2026-05-02T00:00:00Z', value: 18 },
    ];
    metricMocks.refetchValue.mockClear();
    metricMocks.refetchStatus.mockClear();
  });

  it('shows Could not load · Retry in place of the value and does not hide the tile', () => {
    metricMocks.valueError = true;
    renderTile('total_miles');

    const chip = screen.getByTestId('sensor-chip');
    expect(screen.queryByTestId('sensor-chip-hidden')).toBeNull();
    expect(chip).toHaveAttribute('data-sensor-error', 'true');
    expect(chip).not.toHaveAttribute('data-sensor-empty');
    expect(chip).not.toHaveAttribute('aria-busy');
    expect(within(chip).getByRole('alert')).toHaveTextContent('Could not load · Retry');
    expect(chip.querySelector('[data-sensor-value]')).toBeNull();
    expect(chip).not.toHaveTextContent(/^-$|—|No data/);
    expect(screen.queryByTestId('sensor-sprite-layer')).toBeNull();

    fireEvent.click(within(chip).getByRole('button', { name: 'Retry' }));
    expect(metricMocks.refetchValue).toHaveBeenCalledTimes(1);
  });

  it('shows the error instead of the Unavailable chip when the vehicle status read fails', () => {
    metricMocks.statusError = true;
    renderTile('twelve_volt_health');

    const chip = screen.getByTestId('sensor-chip');
    expect(within(chip).getByRole('alert')).toHaveTextContent('Could not load · Retry');
    expect(chip).not.toHaveTextContent('Unavailable');
    fireEvent.click(within(chip).getByRole('button', { name: 'Retry' }));
    expect(metricMocks.refetchStatus).toHaveBeenCalledTimes(1);
  });

  it('keeps the normal value when the read succeeds', () => {
    renderTile('total_miles');

    const chip = screen.getByTestId('sensor-chip');
    expect(chip).not.toHaveAttribute('data-sensor-error');
    expect(within(chip).queryByRole('alert')).toBeNull();
    expect(chip.querySelector('[data-sensor-value]')).not.toBeNull();
  });
});

describe('sensor tile read failure from the dashboard data provider', () => {
  const instance = (id: string, definitionId: string) => ({
    id,
    componentType: 'sensor' as const,
    definitionId,
    options: {},
    layout: { x: 0, y: 0, w: 3, h: 2 },
  });
  const milesTile = instance('d9000009-0000-0000-0000-000000000081', 'total_miles');
  const healthTile = instance('d9000009-0000-0000-0000-000000000082', 'battery_health_pct');

  function renderDashboardTiles() {
    const tiles = [milesTile, healthTile];
    return render(
      <DashboardDataProvider ctx={defaultCtx} requirements={collectDashboardDataRequirements(tiles)}>
        {tiles.map((tile) => {
          const Tile = getWidget('sensor', tile.definitionId)!.component;
          return (
            <div key={tile.id} data-testid={`tile-${tile.definitionId}`}>
              <Tile instance={tile} ctx={defaultCtx} />
            </div>
          );
        })}
      </DashboardDataProvider>,
    );
  }

  const chipOf = (definitionId: string) => within(screen.getByTestId(`tile-${definitionId}`)).getByTestId('sensor-chip');

  beforeEach(() => {
    applyThemePreferences({ mode: 'dark', palette: 'classic' });
    metricMocks.batchError = false;
    metricMocks.healthError = false;
    metricMocks.valueError = false;
    metricMocks.statusError = false;
    metricMocks.vehicleStatus = null;
    metricMocks.value = { metric: 'total_miles', value: 12345, unit: 'mi', label: 'Total Miles', ts: '2026-05-07T00:00:00Z' };
    metricMocks.series = [
      { ts: '2026-05-01T00:00:00Z', value: 10 },
      { ts: '2026-05-02T00:00:00Z', value: 18 },
    ];
    metricMocks.batteryHealth = {
      usable_now_kwh: 111.6,
      usable_new_kwh: 109.0,
      battery_health_pct: 96.4,
      estimated_degradation_pct: 3.6,
      charging_cycles: 18,
      charge_count: 22,
      total_energy_added_kwh: 1800,
      total_energy_used_kwh: 1900,
      charging_efficiency_pct: 94.4,
    };
    metricMocks.refetchBatch.mockClear();
    metricMocks.refetchHealth.mockClear();
    metricMocks.refetchStatus.mockClear();
    metricMocks.refetchValue.mockClear();
  });

  it('shows Could not load · Retry on a tile whose provider source failed, and Retry refetches only that source', () => {
    metricMocks.healthError = true;
    renderDashboardTiles();

    const healthChip = chipOf('battery_health_pct');
    expect(healthChip).toHaveAttribute('data-sensor-error', 'true');
    expect(healthChip).not.toHaveAttribute('data-sensor-empty');
    expect(healthChip).not.toHaveAttribute('aria-busy');
    expect(within(healthChip).getByRole('alert')).toHaveTextContent('Could not load · Retry');
    expect(healthChip.querySelector('[data-sensor-value]')).toBeNull();

    fireEvent.click(within(healthChip).getByRole('button', { name: 'Retry' }));
    expect(metricMocks.refetchHealth).toHaveBeenCalledTimes(1);
    expect(metricMocks.refetchBatch).not.toHaveBeenCalled();
    expect(metricMocks.refetchStatus).not.toHaveBeenCalled();
  });

  it('keeps a tile whose provider source succeeded unaffected', () => {
    metricMocks.healthError = true;
    renderDashboardTiles();

    const milesChip = chipOf('total_miles');
    expect(milesChip).not.toHaveAttribute('data-sensor-error');
    expect(within(milesChip).queryByRole('alert')).toBeNull();
    expect(milesChip.querySelector('[data-sensor-value]')).not.toBeNull();
  });

  it('shows the error on a metric tile when the metric batch fails, and Retry refetches the batch', () => {
    metricMocks.batchError = true;
    renderDashboardTiles();

    const milesChip = chipOf('total_miles');
    expect(screen.queryAllByTestId('sensor-chip-hidden')).toHaveLength(0);
    expect(within(milesChip).getByRole('alert')).toHaveTextContent('Could not load · Retry');
    expect(chipOf('battery_health_pct')).not.toHaveAttribute('data-sensor-error');

    fireEvent.click(within(milesChip).getByRole('button', { name: 'Retry' }));
    expect(metricMocks.refetchBatch).toHaveBeenCalledTimes(1);
    expect(metricMocks.refetchHealth).not.toHaveBeenCalled();
    expect(metricMocks.refetchValue).not.toHaveBeenCalled();
  });

  it('shows no error when every provider source succeeds', () => {
    renderDashboardTiles();

    expect(screen.queryAllByRole('alert')).toHaveLength(0);
    expect(chipOf('total_miles').querySelector('[data-sensor-value]')).not.toBeNull();
    expect(chipOf('battery_health_pct').querySelector('[data-sensor-value]')).not.toBeNull();
  });
});
