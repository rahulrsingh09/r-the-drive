import React from 'react';
import { usePhantomDrainPeriods } from '@panda/hooks';
import { BarChart, Card, Chart, CountUp, DataTable, Legend, LineChart, RangeBars, tip } from '../chartKit';
import { Kpi, Spec } from '../controls';
import type { InsightsData } from '../InsightsContext';
import { batteryHealthPct, usableCapacityNow } from '../../../lib/batteryHealth';
import {
  addDays, batteryBand, dayRange, drainByDay, monthDay, n0, n1, n2, sum, weekdayMonthDay, type CapacityDay,
} from '../insightsModel';

/** Parked periods shorter than this are left out. */
const MIN_PARK_HOURS = 3;

export function BatteryTab({ data, capacity }: { data: InsightsData; capacity: CapacityDay[] }) {
  const { charges, trips, first, today, status, vehicleId, timezone } = data;
  const from30 = addDays(today, -29) > first ? addDays(today, -29) : first;
  const span = React.useMemo(() => ({ from: new Date(`${from30}T00:00:00Z`).getTime() - 864e5, to: Date.now() }), [from30]);
  const parks = usePhantomDrainPeriods(vehicleId, new Date(span.from).toISOString(), new Date(span.to).toISOString(), 250, MIN_PARK_HOURS);
  const days = dayRange(first, today);
  const band = batteryBand(days, trips, charges);
  const limit = status?.battery_limit ?? null;
  const kwh = sum(charges, c => c.kwh);

  const capNow = usableCapacityNow(capacity);
  const cap0 = capacity[0]?.kwh ?? null;
  const capValues = capacity.map(c => c.kwh);
  const cmin = capValues.length ? Math.min(...capValues) : 0;
  const cmax = capValues.length ? Math.max(...capValues) : 0;
  const health = batteryHealthPct(capacity);
  const lost = capNow != null && cap0 != null ? cap0 - capNow : 0;
  const level = status?.battery_level ?? null;
  const range = status?.range_miles ?? null;
  const full = level && range != null && level > 0 ? range / (level / 100) : null;

  const drain = drainByDay(parks.data?.periods ?? [], timezone);
  const days30 = dayRange(from30, today);
  const drainDays = days30.filter(d => drain.has(d));
  const parkedHours = sum(drainDays, d => drain.get(d)!.hours);
  const parkedLoss = sum(drainDays, d => drain.get(d)!.loss);
  const perDay = parkedHours ? parkedLoss / (parkedHours / 24) : null;
  const drainMax = Math.max(1, ...drainDays.map(d => drain.get(d)!.loss));

  return (
    <>
      <section className="ri-card ri-pad ri-rise ri-wide" aria-label="Battery health">
        <Kpi label="Battery health" aside={health == null ? 'Not reported' : <>{lost > 0.05 ? `${n1(lost)} kWh below` : 'Same as'} the first day<br />
          {lost > 0.05 && cap0 ? `${n1(lost / cap0 * 100)}% degradation` : 'No degradation'}</>}>
          {health == null ? <span className="ri-muted">–</span> : <><CountUp value={health} decimals={1} /><small>%</small></>}
        </Kpi>
        {health != null && <div className="ri-hbar" role="presentation"><div className="ri-hbar-fill" style={{ width: `${Math.min(100, health)}%` }} /></div>}
        <Spec className="ri-spec-in" items={[
          { label: 'Usable capacity now', value: capNow != null ? `${n1(capNow)} kWh` : 'Not reported' },
          { label: 'On the first day', value: cap0 != null ? `${n1(cap0)} kWh` : 'Not reported' },
          { label: 'Range at 100%', value: full != null ? `${n0(full)} mi` : 'Not reported', note: full != null ? `car estimate, from ${n0(range!)} mi at ${n0(level!)}%` : undefined },
          { label: 'Full charge cycles', value: capNow ? n1(kwh / capNow) : '–' },
          { label: 'Charges', value: charges.length },
          { label: 'Energy added', value: `${n0(kwh)} kWh` },
        ]} />
        <p className="ri-note">
          Health compares today’s usable capacity with the first day of data ({monthDay(capacity[0]?.day ?? first)}). The factory figure is not in the data.
          {capValues.length > 1 && ` Day-to-day readings move between ${n1(cmin)} and ${n1(cmax)} kWh, so a change under about 1 kWh is noise, not wear.`}
        </p>
      </section>

      <Card title="Usable capacity" sub="the car reports, by day" index={1}>
        {capacity.length ? <Chart title="Usable capacity" render={(w, big) => (
          <LineChart width={w} big={big} height={130} label="Usable capacity the car reports, by day" color="var(--ri-home)"
            points={capacity.map((c, i) => ({ x: i, y: c.kwh, tip: tip(`${n1(c.kwh)} kWh`, monthDay(c.day)) }))}
            yDomain={[Math.floor(cmin) - 1, Math.ceil(cmax) + 1]} ticks={[Math.floor(cmin) - 1, Math.ceil(cmax) + 1]} tickFormat={v => `${v} kWh`}
            endLabels={[[0, n1(capacity[0]!.kwh), 'start'], [-1, n1(capacity[capacity.length - 1]!.kwh), 'end']]}
            xLabels={[[0, monthDay(capacity[0]!.day), 'start'], [capacity.length - 1, 'Today', 'end']]} />
        )} table={() => <DataTable caption="Usable capacity by day" head={['Day', 'Usable capacity']}
          rows={[...capacity].reverse().map(c => [weekdayMonthDay(c.day), `${n1(c.kwh)} kWh`])} />} />
          : <p className="ri-empty">The car has not reported its usable capacity yet.</p>}
      </Card>

      <Card title="Battery level each day" sub="lowest to highest" index={2}>
        <Legend items={[{ label: 'Daily range', color: 'var(--ri-acc)' }, ...(limit != null ? [{ label: `Limit ${n0(limit)}%`, color: 'var(--ri-t2)', line: true, dashed: true }] : [])]} />
        {band.length ? <Chart title="Battery level each day" render={(w, big) => (
          <RangeBars width={w} big={big} height={160} label="Daily battery range" count={days.length} limit={limit}
            startLabel={monthDay(days[0]!)} endLabel="Today"
            bars={band.map(b => ({ index: b.index, lo: b.lo, hi: b.hi, color: 'var(--ri-acc)', tip: tip(weekdayMonthDay(b.day), `${n0(b.lo)}% – ${n0(b.hi)}%`) }))} />
        )} table={() => <DataTable caption="Battery level each day" head={['Day', 'Lowest', 'Highest']}
          rows={[...band].reverse().map(b => [weekdayMonthDay(b.day), `${n0(b.lo)}%`, `${n0(b.hi)}%`])} />} />
          : <p className="ri-empty">No battery readings yet.</p>}
      </Card>

      <section className="ri-card ri-pad ri-rise ri-wide" style={{ '--i': 3 } as React.CSSProperties} aria-label="Parked loss, last 30 days">
        <Kpi label="Parked loss · last 30 days" aside={<>{n0(parkedHours / 24)} days parked<br />{n1(parkedLoss)}% lost in total</>}>
          {perDay == null ? <span className="ri-muted">–</span> : <><CountUp value={perDay} decimals={2} /><small>% / day</small></>}
        </Kpi>
        <div className="ri-gap">
          <Chart title="Parked loss, last 30 days" render={(w, big) => (
            <BarChart width={w} big={big} height={120} label="Battery lost while parked, per day, last 30 days" max={drainMax}
              bars={days30.map((d, i) => {
                const day = drain.get(d);
                return { label: i === 0 ? monthDay(d) : i === days30.length - 1 ? 'Today' : '',
                  segs: [{ v: day?.loss ?? 0, color: 'var(--ri-acc)', tip: tip(weekdayMonthDay(d), day ? `${n1(day.loss)}% over ${n0(day.hours)} h parked` : 'No long park') }] };
              })} />
          )} table={() => <DataTable caption="Parked loss per day" head={['Day', 'Parked', 'Lost', 'Per day']}
            rows={[...drainDays].reverse().map(d => { const day = drain.get(d)!; return [weekdayMonthDay(d), `${n0(day.hours)} h`, `${n1(day.loss)}%`, `${n2(day.loss / (day.hours / 24))}%`]; })} />} />
        </div>
        <p className="ri-note">Battery lost while parked for {MIN_PARK_HOURS} hours or more, grouped by the day parking started.{perDay != null && perDay < 0.5 ? ' This is very low.' : ''}</p>
      </section>
    </>
  );
}
