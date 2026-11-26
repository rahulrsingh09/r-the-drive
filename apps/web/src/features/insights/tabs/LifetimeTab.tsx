import React from 'react';
import { useVehicleHealth } from '@panda/hooks';
import { formatAppDate, formatAppTime } from '@panda/ui/lib/dateTime';
import { BarChart, Card, Chart, CountUp, DataTable, LineChart, tip } from '../chartKit';
import { Kpi, Spec } from '../controls';
import type { InsightsData } from '../InsightsContext';
import { byMonth, dayRange, daysBetween, inWeek, monthDay, monthName, n0, n1, odometerGaps, sum, usd, weeksFrom } from '../insightsModel';

export function LifetimeTab({ data }: { data: InsightsData }) {
  const { trips, charges, first, today, status, vehicleId, timezone } = data;
  const health = useVehicleHealth(vehicleId);
  const versions = new Set((health.data?.software_history ?? []).map(entry => entry.version).filter(Boolean));
  const updates = versions.size > 1 ? versions.size - 1 : 0;
  const miles = sum(trips, t => t.miles);
  const longest = trips.reduce<typeof trips[number] | null>((a, b) => (!a || b.miles > a.miles ? b : a), null);
  const dc = charges.filter(c => c.dc).length;
  const cost = sum(charges, c => c.cost);
  const unpriced = charges.filter(c => c.cost == null).length;
  const days = daysBetween(first, today);

  const odoStart = trips.find(t => t.odoStart != null)?.odoStart ?? null;
  const odoNow = status?.odometer_miles ?? [...trips].reverse().find(t => t.odoEnd != null)?.odoEnd ?? null;
  const allDays = dayRange(first, today);
  let reading = odoStart ?? 0;
  const odoPoints = odoStart == null ? [] : allDays.map((day, i) => {
    trips.filter(t => t.day === day).forEach(t => { if (t.odoEnd != null) reading = t.odoEnd; });
    if (i === allDays.length - 1 && odoNow != null) reading = Math.max(reading, odoNow);
    return { x: i, y: reading, tip: tip(`${n0(reading)} mi odometer`, monthDay(day)) };
  });
  const odoMiles = odoStart != null && odoNow != null && odoNow >= odoStart ? odoNow - odoStart : null;
  const gaps = odometerGaps(trips, timezone);
  const weeks = weeksFrom(first, today);
  const cover = odoMiles != null && odoMiles > 0 ? Math.min(100, miles / odoMiles * 100) : null;
  const months = byMonth(trips, charges);
  const lo = odoStart != null ? Math.floor(odoStart / 100) * 100 : 0;
  const hi = odoNow != null ? Math.ceil(odoNow / 100) * 100 + 100 : lo + 100;

  return (
    <>
      <section className="ri-card ri-pad ri-rise ri-wide" aria-label="Driven since the first day">
        <Kpi label={`Driven since ${monthDay(first)}`} aside={<>{monthDay(first)} – today<br />{days} {days === 1 ? 'day' : 'days'}</>}>
          <CountUp value={miles} /><small>mi</small>
        </Kpi>
      </section>

      <Spec className="ri-rise ri-wide" index={1} items={[
        { label: 'Trips', value: trips.length },
        { label: 'Longest trip', value: longest ? `${n0(longest.miles)} mi` : '–', note: longest?.title },
        { label: 'Energy charged', value: `${n0(sum(charges, c => c.kwh))} kWh` },
        { label: 'Charges', value: `${charges.length - dc} AC · ${dc} DC` },
        { label: 'Spent', value: usd(cost), note: unpriced ? `${unpriced} not set` : undefined },
        { label: 'Software updates', value: versions.size ? updates : 'Not reported' },
      ]} />

      <Card title="Odometer" sub="end of each day" index={2}>
        {odoPoints.length ? <Chart title="Odometer" render={(w, big) => (
          <LineChart width={w} big={big} label="Odometer reading at the end of each day" color="var(--ri-home)" area points={odoPoints}
            yDomain={[lo, hi]} ticks={[lo, hi]} tickFormat={v => `${n0(v)} mi`}
            endLabels={[[0, n0(odoStart!), 'start'], [-1, n0(odoPoints[odoPoints.length - 1]!.y), 'end']]}
            xLabels={[[0, monthDay(first), 'start'], [allDays.length - 1, 'Today', 'end']]} />
        )} table={() => <DataTable caption="Odometer by day" head={['Day', 'Odometer']}
          rows={[...odoPoints].reverse().map(p => [monthDay(allDays[p.x]!), `${n0(p.y)} mi`])} />} />
          : <p className="ri-empty">No trip has an odometer reading yet.</p>}
      </Card>

      <section className="ri-card ri-pad ri-rise" style={{ '--i': 3 } as React.CSSProperties} aria-label="Trip coverage">
        <Kpi label="Trips cover">
          {cover == null ? <span className="ri-muted">–</span> : <><CountUp value={cover} decimals={1} /><small>% of odometer miles</small></>}
        </Kpi>
        {odoMiles != null && <p className="ri-note">
          The odometer moved {n1(odoMiles)} mi. Recorded trips add up to {n1(miles)} mi.
          {odoMiles - miles > 0.05 ? ` The other ${n1(odoMiles - miles)} mi are ${gaps.length} ${gaps.length === 1 ? 'move' : 'moves'} between trips.` : ''}
        </p>}
        <div className="ri-gap">
          <Chart title="Miles not in a trip, per week" render={(w, big) => (
            <BarChart width={w} big={big} height={110} label="Odometer miles with no trip recorded, per week" bars={weeks.map(week => {
              const v = sum(gaps.filter(g => inWeek(g.day, week)), g => g.miles);
              return { label: monthDay(week), top: v ? n1(v) : '0', zero: !v, segs: [{ v, color: 'var(--ri-office)', tip: tip(`Week of ${monthDay(week)}`, `${n1(v)} mi not in a trip`) }] };
            })} />
          )} table={() => <DataTable caption="Miles not in a trip" head={['From', 'To', 'Miles']}
            rows={[...gaps].reverse().map(g => [`${formatAppDate(g.start)} ${formatAppTime(g.start)}`, formatAppTime(g.end), n1(g.miles)])} />} />
        </div>
      </section>

      <Card title="By month" wide index={4}>
        <DataTable caption="By month" head={['Month', 'Trips', 'Miles', 'Charged']}
          rows={months.map(m => [monthName(m.month), m.trips, n0(m.miles), `${n0(m.kwh)} kWh`])} />
      </Card>
    </>
  );
}
