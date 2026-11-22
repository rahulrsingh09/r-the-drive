import React from 'react';
import { BarChart, Card, Chart, DataTable, Legend, MultiLine, RangeBars, tip, type Series } from '../chartKit';
import { CategoryLegend, Spec } from '../controls';
import { ACTIVITY_PATH, type InsightsData } from '../InsightsContext';
import {
  CATEGORIES, CATEGORY_COLOR, CATEGORY_LABEL, addDays, dayRange, niceTicks, inWeek, mondayOf, monthDay, n0, n1, speedGroups, sum,
  weekdayMonthDay, weeksFrom, type ChargeRow,
} from '../insightsModel';
import { useChargeCurves } from '../useInsightsData';

/** "DC fast · Kelso" for a DC charge, else the place name. */
export function chargeTitle(charge: ChargeRow) {
  return charge.dc ? `DC fast · ${charge.placeName}` : charge.placeName;
}

/** Charges with a curve to read. The newest first, at most this many. */
const CURVE_LIMIT = 150;

export function ChargingTab({ data, capacityKwh }: { data: InsightsData; capacityKwh: number | null }) {
  const { charges, first, today, vehicleId, status } = data;
  const ids = React.useMemo(() => charges.slice(-CURVE_LIMIT).map(c => c.id), [charges]);
  const { curves } = useChargeCurves(vehicleId, ids);
  const kwh = sum(charges, c => c.kwh);
  const dc = charges.filter(c => c.dc);
  const ac = charges.filter(c => !c.dc);
  const weeks = weeksFrom(first, today);
  const limit = status?.battery_limit ?? null;

  const series: Series[] = dc.flatMap((charge, index) => {
    const points = (curves.get(charge.id) ?? [])
      .filter(p => Number.isFinite(p.soc_pct) && Number.isFinite(p.power_kw) && p.power_kw > 0)
      .map(p => ({ x: p.soc_pct, y: p.power_kw, tip: tip(`${n0(p.power_kw)} kW at ${n0(p.soc_pct)}%`, `${monthDay(charge.day)} · ${charge.placeName}`) }));
    return points.length > 1 ? [{ name: `${monthDay(charge.day)} · ${charge.placeName}`, dash: index ? '6 5' : '', points }] : [];
  });
  const curveMax = Math.max(100, ...series.flatMap(s => s.points.map(p => p.y)));
  const curveTop = Math.ceil(curveMax / 50) * 50;
  const socs = series.flatMap(s => s.points.map(p => p.x));
  const socLo = socs.length ? Math.max(0, Math.floor(Math.min(...socs) / 10) * 10) : 0;
  const socHi = socs.length ? Math.min(100, Math.max(socLo + 20, Math.ceil(Math.max(...socs) / 10) * 10)) : 100;
  const socTicks = niceTicks(socLo, socHi, 4).filter(t => t >= socLo && t <= socHi);
  const speed = speedGroups(charges, curves);

  const weekKwh = (week: string, pick: (c: ChargeRow) => boolean) => sum(charges.filter(c => inWeek(c.day, week) && pick(c)), c => c.kwh);

  // Calendar: whole weeks from the first Monday to the Sunday of this week.
  const cells = dayRange(mondayOf(first), addDays(mondayOf(today), 6));
  const dayKwh = new Map<string, number>();
  charges.forEach(c => dayKwh.set(c.day, (dayKwh.get(c.day) ?? 0) + c.kwh));
  const maxDay = Math.max(1, ...dayKwh.values());

  return (
    <>
      <Spec className="ri-rise ri-wide" items={[
        { label: 'Energy added', value: `${n0(kwh)} kWh` },
        { label: 'Charges', value: charges.length, note: `${ac.length} AC · ${dc.length} DC` },
        { label: 'Full charge cycles', value: capacityKwh ? n1(kwh / capacityKwh) : '–', note: capacityKwh ? `energy added ÷ ${n1(capacityKwh)} kWh` : 'usable capacity not reported' },
        { label: 'DC share', value: kwh ? `${n0(sum(dc, c => c.kwh) / kwh * 100)}%` : '–' },
      ]} />

      <Card title="DC charging curve" sub="power by battery level" wide index={1}>
        {series.length ? (
          <>
            <Legend items={series.map(s => ({ label: s.name, color: CATEGORY_COLOR.other, line: true, dashed: !!s.dash }))} />
            <Chart title="DC charging curve" render={(w, big) => (
              <MultiLine width={w} big={big} label="DC charging power by battery level" series={series} xDomain={[socLo, socHi]} yDomain={[0, curveTop]}
                xTicks={socTicks} xUnit="%" yTicks={[0, curveTop / 2, curveTop]} yUnit=" kW" color={CATEGORY_COLOR.other} />
            )} />
          </>
        ) : <p className="ri-empty">No DC fast charge with power readings yet.</p>}
        <p className="ri-note">Power is estimated from the battery-level change where the car did not report it. Charging slows as the battery fills.</p>
      </Card>

      <Card title="Start and end of each charge" sub="battery %" wide index={2} link={{ to: ACTIVITY_PATH, label: 'See every charge', search: { filter: 'charges' } }}>
        <CategoryLegend />
        {charges.length ? <Chart title="Start and end of each charge" render={(w, big) => (
          <RangeBars width={w} big={big} label="Battery level at the start and end of each charge" count={charges.length} limit={limit}
            startLabel={monthDay(charges[0]!.day)} endLabel={monthDay(charges[charges.length - 1]!.day)}
            bars={charges.flatMap((c, index) => c.socStart != null && c.socEnd != null ? [{
              index, lo: Math.min(c.socStart, c.socEnd), hi: Math.max(c.socStart, c.socEnd), color: CATEGORY_COLOR[c.category],
              tip: tip(`${chargeTitle(c)} · ${monthDay(c.day)}`, `${n0(c.socStart)}% → ${n0(c.socEnd)}% · +${n1(c.kwh)} kWh`),
            }] : [])} />
        )} table={() => <DataTable caption="Start and end of each charge" head={['Date', 'Place', 'Start', 'End', 'Added']}
          rows={charges.map(c => [monthDay(c.day), c.placeName, c.socStart != null ? `${n0(c.socStart)}%` : '–', c.socEnd != null ? `${n0(c.socEnd)}%` : '–', `${n1(c.kwh)} kWh`])} />} />
          : <p className="ri-empty">No charges yet.</p>}
        {limit != null && <p className="ri-note">The dashed line is the charge limit, {n0(limit)}%.</p>}
      </Card>

      <Card title="Average charging speed" sub="kW" index={3}>
        <Chart title="Average charging speed" render={(w, big) => (
          <BarChart width={w} big={big} label="Average charging speed" bars={speed.map(g => ({
            label: g.label, top: g.kw ? `${n1(g.kw)} kW` : '', segs: [{ v: g.kw, color: g.color, tip: tip(g.label, `${n1(g.kw)} kW average · ${g.count} ${g.count === 1 ? 'charge' : 'charges'}`) }],
          }))} />
        )} table={() => <DataTable caption="Average charging speed" head={['Group', 'Charges', 'Average']}
          rows={speed.map(g => [g.label, g.count, g.kw ? `${n1(g.kw)} kW` : '–'])} />} />
        <p className="ri-note">Energy added ÷ the time until the battery stopped rising. Time plugged in after that does not count.</p>
      </Card>

      <Card title="AC and DC per week" sub="kWh" index={4}>
        <Legend items={[{ label: 'AC', color: CATEGORY_COLOR.home }, { label: 'DC fast', color: CATEGORY_COLOR.other }]} />
        <Chart title="AC and DC per week" render={(w, big) => (
          <BarChart width={w} big={big} height={140} label="AC and DC energy per week" bars={weeks.map(week => {
            const a = weekKwh(week, c => !c.dc); const d = weekKwh(week, c => c.dc);
            return { label: monthDay(week), top: n0(a + d), segs: [
              { v: a, color: CATEGORY_COLOR.home, tip: tip(`Week of ${monthDay(week)}`, `AC: ${n1(a)} kWh`) },
              { v: d, color: CATEGORY_COLOR.other, tip: tip(`Week of ${monthDay(week)}`, `DC: ${n1(d)} kWh`) },
            ] };
          })} />
        )} table={() => <DataTable caption="AC and DC per week" head={['Week', 'AC', 'DC']}
          rows={weeks.map(week => [monthDay(week), `${n1(weekKwh(week, c => !c.dc))} kWh`, `${n1(weekKwh(week, c => c.dc))} kWh`])} />} />
      </Card>

      <Card title="Charging calendar" sub="size = energy" index={5}>
        <CategoryLegend />
        <div className="ri-cal" role="table" aria-label="Charging calendar">
          <div role="row" className="ri-cal-row">
            {['M', 'T', 'W', 'T', 'F', 'S', 'S'].map((x, i) => <div key={i} role="columnheader" className="ri-wd">{x}</div>)}
          </div>
          {Array.from({ length: Math.ceil(cells.length / 7) }, (_, row) => (
            <div role="row" key={row} className="ri-cal-row">
              {cells.slice(row * 7, row * 7 + 7).map((day, col) => {
                const index = row * 7 + col;
                const items = charges.filter(c => c.day === day);
                const k = sum(items, c => c.kwh);
                const out = day < first || day > today;
                const size = k ? 34 + 54 * Math.sqrt(k / maxDay) : 0;
                let acc = 0;
                const cone = CATEGORIES.map(cat => {
                  const share = sum(items.filter(c => c.category === cat), c => c.kwh) / (k || 1) * 360;
                  const seg = share ? `${CATEGORY_COLOR[cat]} ${acc}deg ${acc + share}deg` : '';
                  acc += share; return seg;
                }).filter(Boolean).join(',');
                const text = k ? tip(weekdayMonthDay(day), ...items.map(c => `${chargeTitle(c)}: ${n1(c.kwh)} kWh`)) : undefined;
                return (
                  <div key={day} role="cell" className={`ri-c${out ? ' ri-out' : ''}${k ? ' ri-has' : ''}`} data-tip={text}
                    aria-label={out ? undefined : k ? text!.replace(/\n/g, '. ') : `${weekdayMonthDay(day)}, no charge`}>
                    {k > 0 && <i style={{ '--i': index, width: `${size}%`, height: `${size}%`, background: `conic-gradient(${cone})` } as React.CSSProperties} />}
                    <span aria-hidden="true">{out ? '' : Number(day.slice(8))}</span>
                  </div>
                );
              })}
            </div>
          ))}
        </div>
      </Card>

      <Card title="Energy charged per week" sub="by place, kWh" index={6}>
        <CategoryLegend />
        <Chart title="Energy charged per week" render={(w, big) => (
          <BarChart width={w} big={big} label="Energy charged per week by place" bars={weeks.map(week => ({
            label: monthDay(week), top: n0(weekKwh(week, () => true)),
            segs: CATEGORIES.map(cat => { const v = weekKwh(week, c => c.category === cat); return { v, color: CATEGORY_COLOR[cat], tip: tip(`Week of ${monthDay(week)}`, `${CATEGORY_LABEL[cat]}: ${n1(v)} kWh`) }; }),
          }))} />
        )} table={() => <DataTable caption="Energy charged per week" head={['Week', 'Home', 'Office', 'Other']}
          rows={weeks.map(week => [monthDay(week), ...CATEGORIES.map(cat => n1(weekKwh(week, c => c.category === cat)))])} />} />
      </Card>
    </>
  );
}
