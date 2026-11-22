import React from 'react';
import { getUnitPreferences } from '@panda/ui/lib/utils';
import { EFFICIENCY_UNITS, EFFICIENCY_UNIT_ORDER, useEfficiencyUnit, type EfficiencyUnit } from '../../r-experience/efficiencyUnit';
import { BarChart, Card, Chart, CountUp, DataTable, LineChart, Scatter, tip } from '../chartKit';
import { Kpi, Seg } from '../controls';
import { ACTIVITY_PATH, type InsightsData } from '../InsightsContext';
import {
  SHORT_TRIP_MI, TRIP_LENGTH_BUCKETS, averageWhMi, cToF, inWeek, measured, monthDay, n0, n1, niceTicks, paddedDomain, sum, weeksFrom,
} from '../insightsModel';

const NICE_STEP: Record<EfficiencyUnit, number> = { wh_mi: 100, mi_kwh: 1, kwh_100mi: 10 };
const UNIT_OPTIONS = EFFICIENCY_UNIT_ORDER.map(u => [u, EFFICIENCY_UNITS[u].label] as const);

export function EfficiencyTab({ data }: { data: InsightsData }) {
  const { trips, first, today } = data;
  const { unit, setUnit, spec } = useEfficiencyUnit();
  const value = (whMi: number) => spec.fromWhMi(whMi);
  const text = (whMi: number | null) => (whMi == null ? '–' : value(whMi).toFixed(spec.decimals));
  const withUnit = (whMi: number | null) => (whMi == null ? '–' : `${text(whMi)} ${spec.label}`);
  const fahrenheit = getUnitPreferences().temperature_unit !== 'celsius';
  const temp = (c: number) => (fahrenheit ? cToF(c) : c);
  const tempUnit = fahrenheit ? ' °F' : ' °C';

  const T = measured(trips);
  const total = averageWhMi(trips);
  const weekly = weeksFrom(first, today)
    .map(week => { const items = T.filter(t => inWeek(t.day, week)); return { week, wh: averageWhMi(items), miles: sum(items, t => t.miles) }; })
    .filter((x): x is { week: string; wh: number; miles: number } => x.wh != null);
  const weeklyY = weekly.map(x => value(x.wh));

  const withTemp = T.filter(t => t.tempC != null);
  const sortedY = withTemp.map(t => value(t.whMi!)).sort((a, b) => a - b);
  const q = (p: number) => sortedY[Math.floor(p * (sortedY.length - 1))] ?? 0;
  const step = NICE_STEP[unit];
  const ylo = Math.floor(q(0.02) / step) * step;
  const yhi = Math.max(ylo + step, Math.ceil(q(0.98) / step) * step);
  const yTicks = [ylo, Number(((ylo + yhi) / 2).toFixed(2)), yhi];
  const temps = withTemp.map(t => temp(t.tempC!)).sort((a, b) => a - b);
  const xTicks = temps.length ? niceTicks(temps[0]!, temps[temps.length - 1]!, 5) : [0, 1];
  const xDomain: [number, number] = [xTicks[0]!, xTicks[xTicks.length - 1]!];

  // Notes from the data: the warmer and the colder half of the trips, split at the median temperature.
  const medianC = withTemp.length ? [...withTemp].sort((a, b) => a.tempC! - b.tempC!)[Math.floor(withTemp.length / 2)]!.tempC! : null;
  const warm = medianC == null ? [] : withTemp.filter(t => t.tempC! >= medianC);
  const cold = medianC == null ? [] : withTemp.filter(t => t.tempC! < medianC);
  const tempNote = warm.length >= 3 && cold.length >= 3
    ? `Trips at ${n0(temp(medianC!))}${tempUnit} or warmer averaged ${withUnit(averageWhMi(warm))}. Colder trips averaged ${withUnit(averageWhMi(cold))}.`
    : '';

  const buckets = TRIP_LENGTH_BUCKETS.map(([label, lo, hi]) => {
    const items = T.filter(t => t.miles >= lo && t.miles < hi);
    return { label, count: items.length, wh: averageWhMi(items) };
  });
  const filled = buckets.filter(b => b.wh != null);
  const most = filled.length > 1 ? filled.reduce((a, b) => (b.wh! > a.wh! ? b : a)) : null;
  const least = filled.length > 1 ? filled.reduce((a, b) => (b.wh! < a.wh! ? b : a)) : null;

  return (
    <>
      <section className="ri-card ri-pad ri-rise ri-wide" aria-label="Average efficiency">
        <Kpi label="Average efficiency" aside={<>{T.length} {T.length === 1 ? 'trip' : 'trips'}<br />{spec.lowerIsBetter ? 'Lower is better.' : 'Higher is better.'}</>}>
          {total == null ? <span className="ri-muted">–</span> : <><CountUp key={unit} value={value(total)} decimals={spec.decimals} /><small>{spec.label}</small></>}
        </Kpi>
        <Seg small className="ri-unitseg" label="Efficiency unit" value={unit} options={UNIT_OPTIONS} onChange={setUnit} />
        <div className="ri-gap">
          {weekly.length ? <Chart title="Efficiency per week" render={(w, big) => (
            <LineChart width={w} big={big} label="Efficiency per week" color="var(--ri-home)" dots
              points={weekly.map((x, i) => ({ x: i, y: value(x.wh), tip: tip(withUnit(x.wh), `Week of ${monthDay(x.week)} · ${n0(x.miles)} mi`) }))}
              yDomain={paddedDomain(weeklyY)}
              endLabels={weekly.length > 1 ? [[0, text(weekly[0]!.wh), 'start'], [-1, text(weekly[weekly.length - 1]!.wh), 'end']] : [[0, text(weekly[0]!.wh), 'start']]}
              xLabels={weekly.map((x, i) => [i, monthDay(x.week), i === 0 ? 'start' : i === weekly.length - 1 ? 'end' : 'middle'] as [number, string, 'start' | 'middle' | 'end'])}
              xDomain={weekly.length > 1 ? undefined : [-1, 1]} />
          )} table={() => <DataTable caption="Efficiency per week" head={['Week', spec.label, 'Miles']}
            rows={weekly.map(x => [monthDay(x.week), text(x.wh), n0(x.miles)])} />} />
            : <p className="ri-empty">No trip of {SHORT_TRIP_MI} mi or more with an energy reading yet.</p>}
        </div>
        <p className="ri-note">Trips under {SHORT_TRIP_MI} miles are left out. Their battery readings are too coarse to measure.</p>
      </section>

      <Card title="Efficiency and outside temperature" sub="each dot is a trip" index={1}>
        {withTemp.length ? <Chart title="Efficiency and outside temperature" render={(w, big) => (
          <Scatter width={w} big={big} label="Efficiency by outside temperature" color="var(--ri-home)" xDomain={xDomain} yDomain={[yTicks[0]!, yTicks[2]!]}
            xTicks={xTicks} xUnit={tempUnit} yTicks={yTicks}
            points={withTemp.map(t => ({
              x: temp(t.tempC!), y: Math.max(yTicks[0]!, Math.min(yTicks[2]!, value(t.whMi!))), r: 3 + Math.min(4, Math.sqrt(t.miles) * 0.7),
              tip: tip(`${withUnit(t.whMi)} · ${n0(temp(t.tempC!))}${tempUnit}`, `${t.title} · ${n1(t.miles)} mi`),
            }))} />
        )} table={() => <DataTable caption="Efficiency by outside temperature" head={['Trip', 'Temperature', spec.label]}
          rows={withTemp.map(t => [t.title, `${n0(temp(t.tempC!))}${tempUnit}`, text(t.whMi)])} />} />
          : <p className="ri-empty">No trip has an outside temperature yet.</p>}
        <p className="ri-note">{tempNote ? `${tempNote} ` : ''}Larger dots are longer trips.</p>
      </Card>

      <Card title="Efficiency by trip length" sub={spec.label} index={2} link={{ to: ACTIVITY_PATH, label: 'See every trip', search: { filter: 'trips' } }}>
        <Chart title="Efficiency by trip length" render={(w, big) => (
          <BarChart width={w} big={big} label="Efficiency by trip length" bars={buckets.map(b => ({
            label: b.label, top: b.wh != null ? text(b.wh) : '',
            segs: [{ v: b.wh != null ? value(b.wh) : 0, color: 'var(--ri-home)', tip: tip(b.label, `${withUnit(b.wh)} · ${b.count} ${b.count === 1 ? 'trip' : 'trips'}`) }],
          }))} />
        )} table={() => <DataTable caption="Efficiency by trip length" head={['Trip length', 'Trips', spec.label]}
          rows={buckets.map(b => [b.label, b.count, text(b.wh)])} />} />
        {most && least && most !== least && <p className="ri-note">{most.label} trips used the most energy per mile. {least.label} trips used the least.</p>}
      </Card>
    </>
  );
}
