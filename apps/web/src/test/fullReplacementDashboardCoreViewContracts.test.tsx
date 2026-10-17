import React from 'react';
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { WidgetHost, WidgetPresentationProvider } from '../../../../packages/dashboards/src/WidgetHost';
import { registerWidget } from '../../../../packages/dashboards/src/registry';
import type { WidgetInstance } from '../../../../packages/dashboards/src/schema';
import {
  getChartDefinition, getChartSettingsCapabilities,
  type DashboardChartSource,
} from '../../../../packages/dashboards/src/charts/catalog';

const ctx = { vehicleId: null, from: null, to: null };
function fixture(componentType: WidgetInstance['componentType'], definitionId = 'view-contract'): WidgetInstance {
  registerWidget({
    componentType, definitionId, title: 'Registry title',
    defaultSize: { w: 3, h: 2 }, minSize: { w: 1, h: 1 },
    component: () => <span>Widget content</span>,
  });
  return { id: 'synthetic', componentType, definitionId, title: 'Saved title', layout: { x: 0, y: 0, w: 3, h: 2 } };
}

describe('widget host view contract', () => {
  it.each([
    ['chart', 'view-contract', true], ['sensor', 'view-contract', false],
    ['battery', 'view-contract', false], ['charging', 'view-contract', false],
    ['custom', 'charging.connection', false], ['custom', 'trips.stat', false],
    ['custom', 'view-contract', true],
  ] as const)('preserves title ownership for %s/%s', (type, definition, title) => {
    const { container } = render(<WidgetHost instance={fixture(type, definition)} ctx={ctx} />);
    expect(Boolean(screen.queryByText('Saved title'))).toBe(title);
    expect(container.firstElementChild).toHaveClass('flex', 'h-full', 'min-h-0', 'flex-col');
    expect(container.firstElementChild).toHaveAttribute('data-widget-type', type);
    expect(container.firstElementChild).toHaveAttribute('data-widget-definition', definition);
    expect(screen.getByText('Widget content').parentElement).toHaveClass('min-h-0', 'flex-1', '[&>*]:h-full');
  });

  it('honors picker visibility and chart identity without changing the saved instance', () => {
    const instance = fixture('chart');
    instance.options = { showPicker: false, chartId: 'soc-history' };
    const before = structuredClone(instance);
    const view = render(<WidgetHost instance={instance} ctx={ctx} />);
    expect(screen.queryByText('Saved title')).toBeNull();
    expect(view.container.firstElementChild).toHaveAttribute('data-chart-id', 'soc-history');
    view.rerender(<WidgetHost instance={{ ...instance, title: '', options: { chartId: 12 } }} ctx={ctx} />);
    expect(screen.queryByText('Saved title')).toBeNull();
    expect(view.container.firstElementChild).not.toHaveAttribute('data-chart-id');
    expect(instance).toEqual(before);
  });

  it('keeps unknown widgets in the existing fallback frame without invoking a presentation', () => {
    const instance = { ...fixture('custom'), definitionId: 'missing-definition' };
    const { container } = render(
      <WidgetPresentationProvider value={{ 'custom:missing-definition': () => <span>Unexpected presentation</span> }}>
        <WidgetHost instance={instance} ctx={ctx} />
      </WidgetPresentationProvider>,
    );
    expect(container.innerHTML).toBe('<div class="flex h-full items-center justify-center rounded-lg border border-dashed border-border text-xs text-fg-tertiary">Unknown widget: custom/missing-definition</div>');
  });

  it('composes the keyed presentation around the widget and preserves its state on context updates', () => {
    const instance = fixture('custom');
    function Counter() {
      const [count, setCount] = React.useState(0);
      return <button onClick={() => setCount(count + 1)}>Count {count}</button>;
    }
    registerWidget({ componentType: 'custom', definitionId: instance.definitionId, title: '', defaultSize: { w: 3, h: 2 }, minSize: { w: 1, h: 1 }, component: Counter });
    const presentations = {
      'custom:view-contract': ({ children }: { children: React.ReactNode }) => <section aria-label="Presentation">{children}</section>,
      'chart:view-contract': () => <span>Wrong presentation</span>,
    };
    const tree = (vehicleId: string | null) => <WidgetPresentationProvider value={presentations}>
      <WidgetHost instance={instance} ctx={{ ...ctx, vehicleId }} />
    </WidgetPresentationProvider>;
    const view = render(tree(null));
    fireEvent.click(screen.getByRole('button', { name: 'Count 0' }));
    view.rerender(tree('synthetic-vehicle'));
    expect(screen.getByRole('button', { name: 'Count 1' }).parentElement).toHaveAttribute('aria-label', 'Presentation');
    expect(screen.queryByText('Wrong presentation')).toBeNull();
    expect(screen.getByText('Saved title').nextElementSibling).toHaveClass('min-h-0', 'flex-1', '[&>*]:h-full');
  });
});

describe('chart settings capability selection', () => {
  it.each([
    ['soc_history', true, 'dashboard-timeframe', { y: { label: 'State of Charge', unit: 'synthetic-unit' } }],
    ['charging_sessions_energy', false, 'dashboard-timeframe', {}],
    ['charging_weekly_energy', false, 'dashboard-timeframe', { y: { label: 'Energy charged', unit: 'synthetic-unit' } }],
    ['charge_session_curve', true, 'chart-local', { y: { label: 'Charge rate', unit: 'kW' }, y2: { label: 'Energy added', unit: 'kWh' } }],
    ['charging_curve_analysis', false, 'chart-local', { x: { label: 'Charge level', unit: '%' }, y: { label: 'Charge rate', unit: 'kW' } }],
    ['efficiency_trend', true, 'dashboard-timeframe', { y: { label: 'Efficiency' } }],
    ['efficiency_temperature', false, 'dashboard-timeframe', {}],
    ['efficiency_mode', false, 'dashboard-timeframe', {}],
    ['efficiency_tags', false, 'dashboard-timeframe', {}],
    ['phantom_drain', false, 'dashboard-timeframe', { y: { label: 'Drain rate', unit: 'synthetic-unit' } }],
    ['battery_degradation', true, 'dashboard-timeframe', { y: { label: 'Battery health', unit: 'synthetic-unit' } }],
    ['battery_capacity_mileage', true, 'dashboard-timeframe', { y: { label: 'Usable capacity', unit: 'synthetic-unit' }, y2: { label: 'Mileage', unit: 'mi' } }],
    ['projected_range_mileage', true, 'dashboard-timeframe', { y: { label: 'Projected max range', unit: 'synthetic-unit' }, y2: { label: 'Mileage', unit: 'mi' } }],
    ['tire_pressure_trips', false, 'dashboard-timeframe', { y: { label: 'Tire pressure', unit: 'synthetic-unit' }, y2: { label: 'Trips', unit: 'Trips' } }],
    ['unknown-source', false, 'dashboard-timeframe', {}],
  ] as const)('resolves %s with independent filter, domain, and axis policies', (source, timeFilter, xDomainSource, axes) => {
    const definition = { ...getChartDefinition('soc-history')!, source: source as DashboardChartSource, yUnit: 'synthetic-unit' };
    expect(getChartSettingsCapabilities(definition)).toEqual({ timeFilter, axes, xDomainSource });
    const withoutUnits = JSON.parse(JSON.stringify(axes).replaceAll(',"unit":"synthetic-unit"', ''));
    expect(getChartSettingsCapabilities({ ...definition, yUnit: '' })).toEqual({ timeFilter, axes: withoutUnits, xDomainSource });
    const first = getChartSettingsCapabilities(definition);
    if (first.axes.y) first.axes.y.label = 'changed';
    expect(getChartSettingsCapabilities(definition)).toEqual({ timeFilter, axes, xDomainSource });
  });
});
