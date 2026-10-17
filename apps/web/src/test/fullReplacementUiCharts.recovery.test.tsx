import React from 'react';
import { filterTimeSeriesValues } from '../../../../packages/ui/src/charts/timeFilter';
import { cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { RichTimeSeriesChart, getRichTimeIntervalBandGeometry } from '../../../../packages/ui/src/charts/RichTimeSeriesChart';
import { ChartColorField } from '../../../../packages/ui/src/charts/ChartColorField';
import { DailyChargeSessionsChart } from '../../../../packages/ui/src/charts/DailyChargeSessionsChart';
import { capacityTooltip, elapsedTripLabel, elapsedProfileLabel, nullableProjection, thresholdColor } from '../../../../packages/ui/src/charts/chartPresentation';
import { CHART_COLORS } from '../../../../packages/ui/src/charts/ChartProvider';
import { measureChartLabels } from '../../../../packages/ui/src/charts/chartLabelLayout';

afterEach(() => { cleanup(); vi.restoreAllMocks(); });

describe('recovered chart numeric and control boundaries', () => {
  it('preserves subtraction of emitted readings in the established filter recurrence', () => {
    expect(filterTimeSeriesValues([0, 300000, 600000, 900000], [1, 10, 100, 1000], '15m')).toEqual([5.5, 37, 368.5, 534.25]);
  });
  it('keeps strict threshold equality, missing samples and both elapsed-time conventions', () => {
    const bands = [[95, CHART_COLORS.success], [88, CHART_COLORS.warning]] as const;
    expect([96, 95, 88, NaN].map(value => thresholdColor(value, bands, 'above'))).toEqual([CHART_COLORS.success, CHART_COLORS.warning, CHART_COLORS.danger, CHART_COLORS.danger]);
    const readings = [{ ts: 'a', value: null }, { ts: 'b', value: -0 }, { ts: 'c', value: 2 }];
    expect(nullableProjection(readings, row => row.value, row => ({ ts: row.ts }), 'elev', 3.28084)).toEqual([{ ts: 'a', elev: null }, { ts: 'b', elev: -0 }, { ts: 'c', elev: 6.56168 }]);
    expect([60, 60.5, -1].map(elapsedTripLabel)).toEqual(['1:00', '1:00', '-1:00']);
    expect([60, 60.5, -1].map(value => elapsedProfileLabel(value))).toEqual(['1m', '1:0.5', '-1:-1']);
    expect(capacityTooltip(undefined, 'capacity_pct')).toEqual(['0.0%', 'Capacity']);
    expect(capacityTooltip(2, 'usable_kwh')).toEqual(['2.0 kWh', 'Usable']);
  });

  it('retains the 44px interval target when the plot cannot contain it', () => {
    expect(getRichTimeIntervalBandGeometry(99, 3, 10, 20)).toEqual({ bandTop: 24, bandHeight: 6, laneTop: 28, laneHeight: 2, top: 28, height: 4, hitTop: 10, hitHeight: 44 });
    expect(getRichTimeIntervalBandGeometry(-1, 0, 10, -20)).toEqual({ bandTop: 10, bandHeight: 0, laneTop: 10, laneHeight: 0, top: 10, height: 4, hitTop: 10, hitHeight: 44 });
  });

  it('uses deterministic widths if canvas measurement fails mid-batch', () => {
    const measureText = vi.fn().mockReturnValueOnce({ width: 31 }).mockImplementationOnce(() => { throw new Error('synthetic unavailable font'); }).mockReturnValue({ width: NaN });
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue({ measureText } as unknown as CanvasRenderingContext2D);
    expect(measureChartLabels(['abc', 'def', ''])).toEqual([{ text: 'abc', width: 31 }, { text: 'def', width: 22.32 }, { text: '', width: 1 }]);
    expect(HTMLCanvasElement.prototype.getContext).toHaveBeenCalledOnce();
  });

  it('invalidates the chart when field delimiters would collide in the former structure key', () => {
    const points = [{ ts: 0 }, { ts: 1 }];
    const view = render(<RichTimeSeriesChart points={points} xTime={false} series={[{ key: 'a:b', label: 'c', values: [1, 2] }]} />);
    const first = view.container.querySelector('.uplot')!;
    expect(first).not.toBeNull();
    view.rerender(<RichTimeSeriesChart points={points} xTime={false} series={[{ key: 'a', label: 'b:c', values: [1, 2] }]} />);
    expect(view.container.querySelector('.uplot')).not.toBe(first);
    const second = view.container.querySelector('.uplot');
    view.rerender(<RichTimeSeriesChart points={points} xTime={false} series={[{ key: 'a', label: 'b:c', values: [3, 4] }]} />);
    expect(view.container.querySelector('.uplot')).toBe(second);
  });

  it('retains interval activation text and dispatches the selected interval once', () => {
    const choose = vi.fn();
    const interval = { id: 'trip', start: 0, end: 1, label: 'Trip', details: 'Synthetic trip' };
    const view = render(<RichTimeSeriesChart points={[{ ts: 0 }, { ts: 1 }]} series={[{ key: 'x', label: 'X', values: [1, 2] }]} xTime={false} intervals={[interval]} onIntervalClick={choose} />);
    return vi.waitFor(() => {
      const button = view.getByRole('button', { name: 'Trip: Synthetic trip' });
      fireEvent.focus(button);
      expect(view.getByText('Trip\nSynthetic trip', { exact: true, normalizer: value => value })).toBeInTheDocument();
      fireEvent.click(button);
      expect(choose).toHaveBeenCalledOnce();
      expect(choose.mock.calls[0]![0]).toMatchObject(interval);
    });
  });

  it('clears automatic color highlighting while allowing a semantic or series choice', () => {
    const choose = vi.fn();
    const view = render(<ChartColorField value={{ mode: 'token', token: 'series-02' }} automatic={{ active: true, color: 'var(--rm-series-01)', onSelect: vi.fn() }} onChange={choose} />);
    expect(view.getByRole('button', { name: 'Theme accent color' })).toHaveTextContent('Choose a theme accent');
    fireEvent.click(view.getByRole('button', { name: 'Danger' }));
    expect(choose).toHaveBeenLastCalledWith({ mode: 'token', token: 'danger' });
    view.rerender(<ChartColorField value={{ mode: 'token', token: 'series-02' }} onChange={choose} />);
    expect(view.getByRole('button', { name: 'Theme accent color' })).toHaveTextContent('Accent 02');
  });

  it('preserves controlled daily selection and ignores unrelated keyboard activation', () => {
    const choose = vi.fn();
    const daily = [{ day_local: '2024-01-01', day_start: '2024-01-01T00:00:00Z', total_energy_kwh: 20, session_count: 1 }];
    const view = render(<DailyChargeSessionsChart daily={daily} dailySessions={[]} selectedDayLocal="2024-01-01" onDayClick={choose} />);
    const day = view.getAllByRole('button')[0]!;
    fireEvent.keyDown(day, { key: 'Escape' });
    expect(choose).not.toHaveBeenCalled();
    fireEvent.keyDown(day, { key: 'Enter' });
    expect(choose).toHaveBeenLastCalledWith(null);
  });
});
