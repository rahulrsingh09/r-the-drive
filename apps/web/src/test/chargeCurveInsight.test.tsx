import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import type { ChargeCurvePoint } from '@panda/types';
import { ChargeCurveInsightNote, describeChargeCurve } from '../../../../packages/dashboards/src/widgets/chart/chargeCurveInsight';
import { DailyEnergyBarChart } from '../../../../packages/ui/src/charts/DailyChargeSessionsChart';
import { chargeSessionTimeSplits } from '../../../../packages/dashboards/src/widgets/chart/DashboardChartWidget';

function curve(rows: Array<[number, number, number]>, power_method = 'recorded'): ChargeCurvePoint[] {
  return rows.map(([minutes_elapsed, power_kw, soc_pct]) => ({ minutes_elapsed, power_kw, soc_pct, power_method }));
}

// Rivian's per-minute charging graph for the two Nov 6 2026 R2 DC fast charges.
const morning = curve([
  [0, 26.2, 28], [1, 184.0, 31], [2, 186.4, 34], [3, 188.2, 37], [4, 189.5, 41], [5, 182.5, 44],
  [6, 174.7, 48], [7, 166.7, 51], [8, 155.0, 54], [9, 148.6, 57], [10, 144.4, 59], [11, 137.5, 62],
  [12, 132.2, 64], [13, 123.0, 67], [14, 114.5, 69], [15, 116.5, 71],
]);
const afternoon = curve([
  [0, 56.2, 38], [1, 107.5, 40], [2, 106.7, 42], [3, 104.1, 44], [4, 107.1, 46], [5, 92.1, 48],
  [6, 92.1, 49], [7, 89.1, 51], [8, 105.9, 53], [9, 90.9, 55], [10, 71.1, 56], [11, 70.5, 57],
  [12, 71.4, 59], [13, 74.0, 60], [14, 74.7, 61], [15, 105.9, 63], [16, 107.7, 65], [17, 108.1, 67],
  [18, 84.1, 68], [19, 86.5, 70], [20, 86.1, 71],
]);

describe('describeChargeCurve', () => {
  it('reads a steady fall after the peak as a normal battery taper', () => {
    const insight = describeChargeCurve(morning)!;
    expect(insight.pattern).toBe('battery_taper');
    expect(insight.peakKw).toBe(189.5);
    expect(insight.peakSoc).toBe(41);
    expect(insight.endKw).toBe(116.5);
    expect(insight.endSoc).toBe(71);
    expect(insight.recoveries).toBe(0);
  });

  it('reads power that drops and recovers as a charger limit', () => {
    const insight = describeChargeCurve(afternoon)!;
    expect(insight.pattern).toBe('charger_limited');
    expect(insight.recoveries).toBe(2);
    expect(insight.peakKw).toBe(108.1);
    expect(insight.minAfterRampKw).toBe(70.5);
  });

  it('says nothing for AC sessions, estimates or too few points', () => {
    expect(describeChargeCurve(curve([[0, 11.2, 40], [60, 11.4, 50], [120, 11.3, 60]]))).toBeNull();
    expect(describeChargeCurve(curve([[0, 150, 30], [1, 140, 40], [2, 120, 50]], 'soc_delta'))).toBeNull();
    expect(describeChargeCurve(morning.slice(0, 2))).toBeNull();
  });

  it('does not read a cold-pack ramp as a charger limit', () => {
    const insight = describeChargeCurve(curve([[0, 40, 20], [2, 152, 24], [4, 175, 30], [6, 190, 36], [10, 160, 50], [14, 130, 62]]))!;
    expect(insight.recoveries).toBe(0);
    expect(insight.pattern).toBe('battery_taper');
  });

  it('keeps a flat DC curve without a reason', () => {
    const insight = describeChargeCurve(curve([[0, 48, 40], [5, 50, 45], [10, 49, 50], [15, 50, 55]]))!;
    expect(insight.pattern).toBe('steady');
  });
});

describe('ChargeCurveInsightNote', () => {
  it('explains the taper in plain words and marks it as an estimate', () => {
    render(<ChargeCurveInsightNote points={morning} />);
    expect(screen.getByText(/Peak 189.5 kW at 41%/)).toBeTruthy();
    expect(screen.getByText(/Power fell from 189.5 kW at 41% to 116.5 kW at 71% as the battery filled/)).toBeTruthy();
    expect(screen.getByText(/Rivian does not report a reason/)).toBeTruthy();
  });

  it('names the charger as the probable limit for an up-and-down curve', () => {
    render(<ChargeCurveInsightNote points={afternoon} />);
    expect(screen.getByText(/the charger probably limited the rate/)).toBeTruthy();
  });

  it('renders nothing without a DC curve', () => {
    const { container } = render(<ChargeCurveInsightNote points={[]} />);
    expect(container.innerHTML).toBe('');
  });
});

describe('Energy Charged', () => {
  it('stacks each day by charger type when sessions are given', () => {
    const { container } = render(
      <DailyEnergyBarChart
        daily={[{ day_local: '2026-11-06', day_start: '2026-11-06T00:00:00Z', total_energy_kwh: 54, session_count: 2 }]}
        dailySessions={[
          {
            session_id: 'dc-1', day_local: '2026-11-06', day_start: '2026-11-06T00:00:00Z',
            started_at: '2026-11-06T19:47:20Z', energy_added_kwh: 34.8, cost_usd: null,
            charger_type: 'DC', location_name: null,
          },
          {
            session_id: 'ac-1', day_local: '2026-11-06', day_start: '2026-11-06T00:00:00Z',
            started_at: '2026-11-06T04:08:09Z', energy_added_kwh: 19.2, cost_usd: null,
            charger_type: 'AC', location_name: 'Home',
          },
        ]}
      />,
    );
    expect(screen.getByText('AC')).toBeTruthy();
    expect(screen.getByText('DC')).toBeTruthy();
    expect(container.querySelectorAll('[data-testid="daily-charge-segment"]')).toHaveLength(2);
  });
});

describe('chargeSessionTimeSplits', () => {
  it('ticks a 20-minute DC charge every 5 minutes and a long AC charge every 2 hours', () => {
    const start = Date.parse('2026-11-07T00:08:12Z') / 1000;
    const dc = chargeSessionTimeSplits(start, start + 20 * 60);
    expect(dc).toHaveLength(4);
    expect(dc.every((tick) => tick % 300 === 0)).toBe(true);
    const ac = chargeSessionTimeSplits(start, start + 9 * 3600);
    expect(ac.every((tick) => tick % 7200 === 0)).toBe(true);
    expect(ac.length).toBeGreaterThanOrEqual(4);
  });
});
