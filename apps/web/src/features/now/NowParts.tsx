import React from 'react';
import { Link } from '@tanstack/react-router';
import { CircleDot, Thermometer, Zap } from 'lucide-react';
import type { ChargeSession, Place, TrackPoint, Trip, VehicleStatus } from '@panda/types';
import { formatAppDate, formatAppTime } from '@panda/ui/lib/dateTime';
import { useEfficiencyUnit } from '../r-experience/efficiencyUnit';
import { RVehicleTurn } from '../r-experience/RVehicleOrbit';
import { RVehicleLockStatus } from '../r-experience/RVehicleLockStatus';
import { RPageActions } from '../r-experience/RAppLayout';
import type { Vehicle } from '@panda/types';
import { NowBarChart, NowLineChart, paddedDomain } from './NowCharts';
import {
  chargeCategory, chargeTitle, costText, durationText, isShortTrip, lockWord, n0, n1, n2, routePoints, shortDay,
  tireRange, tireText, tripTitle, usd, type ChargeCategory, type MonthSummary, type WeekSummary,
} from './nowModel';
import { useArrived, useCountUp } from './useNowMotion';

export const CATEGORY_COLOR: Record<ChargeCategory, string> = {
  home: 'var(--now-home)', office: 'var(--now-office)', other: 'var(--now-other)',
};

export type ChipState = 'live' | 'stale' | 'waiting' | 'charging' | 'driving';

/** Page header: the car name, one status chip and the update time. */
export function NowHeader({ name, chip, chipState, sub, control }: {
  name: string; chip: React.ReactNode; chipState: ChipState; sub?: React.ReactNode; control?: React.ReactNode;
}) {
  return <header className="now-top">
    <div className="now-top-text">
      <h1>{name}</h1>
      <div className="now-sub">
        <span className="now-chip" data-state={chipState}>
          {chipState === 'charging' ? <Zap className="now-bolt" aria-hidden="true" /> : <span className="now-dot" aria-hidden="true" />}
          {chip}
        </span>
        {sub && <span className="now-sub-text">{sub}</span>}
      </div>
    </div>
    {control}
    <RPageActions />
  </header>;
}

/** A number that counts up from 0 on the first view (motion allowed). */
export function CountUp({ value, decimals = 0, animate }: { value: number; decimals?: number; animate: boolean }) {
  const shown = useCountUp(value, animate) ?? value;
  return <span className="now-num">{shown.toLocaleString('en-US', { minimumFractionDigits: decimals, maximumFractionDigits: decimals })}</span>;
}

/** The car stage. The turn rests on the page; nothing sits on top of the car. */
export function NowStage({ vehicle, children, mode }: { vehicle: Vehicle; children?: React.ReactNode; mode?: 'charging' }) {
  return <div className="now-stage now-car" data-mode={mode}>
    <RVehicleTurn vehicle={vehicle} />
    {children}
  </div>;
}

/** The limit bar: fill to the battery level and a marker at the charge limit. */
export function LimitBar({ battery, limit, animate, charging }: { battery: number; limit: number | null; animate: boolean; charging?: boolean }) {
  const arrived = useArrived(animate);
  const fill = Math.min(100, Math.max(0, battery));
  const label = limit != null ? `Battery ${Math.round(battery)} percent, charge limit ${Math.round(limit)} percent` : `Battery ${Math.round(battery)} percent`;
  return <>
    <div className="now-bar" data-charging={charging || undefined} role="img" aria-label={label}>
      <div className="now-bar-fill" style={{ width: `${arrived ? fill : 0}%` }} data-animate={animate || undefined} />
      {limit != null && <div className="now-bar-limit" style={{ left: `${Math.min(100, Math.max(0, limit))}%` }} />}
    </div>
    <div className="now-bar-cap" aria-hidden="true">
      <span>0</span>
      {limit != null && <span className="now-bar-cap-limit" style={{ left: `${Math.min(92, Math.max(8, limit))}%` }}>Limit {Math.round(limit)}%</span>}
      <span>100</span>
    </div>
  </>;
}

export function BatteryCard({ battery, limit, rangeMi, animate }: { battery: number | null; limit: number | null; rangeMi: number | null; animate: boolean }) {
  return <section className="now-card now-batt now-rise" style={{ '--i': 1 } as React.CSSProperties} aria-label="Battery">
    <div className="now-batt-row">
      <div>
        <div className="now-label">Battery</div>
        {battery == null ? <div className="now-missing">No reading yet</div>
          : <div className="now-big"><CountUp value={Math.round(battery)} animate={animate} /><small>%</small></div>}
      </div>
      <div className="now-range">
        <div className="now-label">Estimated range</div>
        {rangeMi == null ? <div className="now-missing">No reading yet</div>
          : <div className="now-range-v"><CountUp value={Math.round(rangeMi)} animate={animate} /> <small>mi</small></div>}
      </div>
    </div>
    {battery != null && <LimitBar battery={battery} limit={limit} animate={animate} />}
  </section>;
}

/** Doors, tires and cabin. Each opens the Car page. */
export function NowTiles({ status }: { status: VehicleStatus | null }) {
  const lock = lockWord(status);
  const tires = tireText(tireRange(status));
  const cabin = status?.cabin_temp_c;
  return <div className="now-tiles">
    <Link className="now-tile now-rise" style={{ '--i': 2 } as React.CSSProperties} to="/vehicle-health">
      <span className="now-tile-k"><RVehicleLockStatus status={status} />Doors</span>
      <span className="now-tile-v" data-ok={lock === 'Locked' || undefined}>{lock ?? 'Not reported'}</span>
    </Link>
    <Link className="now-tile now-rise" style={{ '--i': 3 } as React.CSSProperties} to="/vehicle-health">
      <span className="now-tile-k"><CircleDot aria-hidden="true" />Tires</span>
      <span className="now-tile-v now-num">{tires ? <>{tires} <small>psi</small></> : 'Not reported'}</span>
    </Link>
    <Link className="now-tile now-rise" style={{ '--i': 4 } as React.CSSProperties} to="/vehicle-health">
      <span className="now-tile-k"><Thermometer aria-hidden="true" />Cabin</span>
      <span className="now-tile-v now-num">{cabin != null && Number.isFinite(cabin) ? `${Math.round(cabin)}°C` : 'Not reported'}</span>
    </Link>
  </div>;
}

/** The trip route as a small picture: start dot, route and end dot. */
export function RouteGlyph({ track, width = 64, height = 52 }: { track: readonly Pick<TrackPoint, 'lat' | 'lng'>[] | undefined; width?: number; height?: number }) {
  const points = routePoints(track, width, height, 8);
  if (!points) return <svg viewBox="0 0 24 24" className="now-glyph-car" aria-hidden="true"><path d="M3 15.5 5 10a3 3 0 0 1 2.8-2h8.4A3 3 0 0 1 19 10l2 5.5V18a1 1 0 0 1-1 1h-1.5a1 1 0 0 1-1-1v-1H6.5v1a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1z" /></svg>;
  const path = points.map(([x, y], index) => `${index ? 'L' : 'M'}${x.toFixed(1)},${y.toFixed(1)}`).join('');
  const [sx, sy] = points[0]!;
  const [ex, ey] = points.at(-1)!;
  return <svg viewBox={`0 0 ${width} ${height}`} className="now-route" aria-hidden="true">
    <path d={path} /><circle cx={sx} cy={sy} r={2.5} className="now-route-start" /><circle cx={ex} cy={ey} r={3.5} className="now-route-end" />
  </svg>;
}

/** Latest: the last trip and the last charge, each opening its Activity detail. */
export function NowLatest({ trip, track, charge, places }: {
  trip: Trip | undefined; track: TrackPoint[] | undefined; charge: ChargeSession | undefined; places: readonly Place[] | undefined;
}) {
  const efficiency = useEfficiencyUnit();
  return <div className="now-latest">
    {trip && <Link className="now-lrow now-rise" style={{ '--i': 5 } as React.CSSProperties} to="/trips/$tripId" params={{ tripId: trip.id }}>
      <span className="now-glyph"><RouteGlyph track={track} /></span>
      <span className="now-lrow-main"><span className="now-ttl">{tripTitle(trip)}</span>
        <span className="now-meta">{shortDay(trip.started_at)} · {formatAppTime(trip.started_at)} · {durationText(trip.duration_min)}</span></span>
      <span className="now-val now-num">{n1(trip.distance_mi)} mi
        <span className="now-val-sub">{isShortTrip(trip) || trip.efficiency_wh_mi == null ? 'Short trip' : efficiency.formatWithUnit(trip.efficiency_wh_mi)}</span></span>
    </Link>}
    {charge && <Link className="now-lrow now-rise" style={{ '--i': 6 } as React.CSSProperties} to="/charging/$sessionId" params={{ sessionId: charge.id }}>
      <span className="now-glyph" style={{ color: CATEGORY_COLOR[chargeCategory(charge, places)] }}><Zap aria-hidden="true" /></span>
      <span className="now-lrow-main"><span className="now-ttl">{chargeTitle(charge)}</span>
        <span className="now-meta">{[shortDay(charge.started_at),
          charge.soc_start != null && charge.soc_end != null ? `${Math.round(charge.soc_start)}% → ${Math.round(charge.soc_end)}%` : null].filter(Boolean).join(' · ')}</span></span>
      <span className="now-val now-num">{charge.energy_added_kwh != null ? `+${n1(charge.energy_added_kwh)} kWh` : 'Not reported'}
        <span className="now-val-sub">{costText(charge)}</span></span>
    </Link>}
  </div>;
}

/** This week: trip miles with daily bars, efficiency, energy charged and cost. */
export function NowWeek({ week, animate }: { week: WeekSummary; animate: boolean }) {
  const efficiency = useEfficiencyUnit();
  return <section className="now-card now-sum now-rise" style={{ '--i': 7 } as React.CSSProperties} aria-label="This week">
    <div className="now-sum-head">
      <div><div className="now-label">Trip</div><div className="now-big"><CountUp value={Math.round(week.miles)} animate={animate} /><small>mi</small></div></div>
      <div className="now-label now-right">{week.trips} {week.trips === 1 ? 'trip' : 'trips'}<br />{formatAppDate(week.from, { year: undefined })} – today</div>
    </div>
    <div className="now-sum-strip">
      <NowBarChart height={70} padBottom={18} label="Trip miles per day this week"
        bars={week.days.map(day => ({ key: day.key, value: day.miles, label: day.label, title: `${day.title}: ${n1(day.miles)} mi`,
          color: day.today ? 'var(--now-acc)' : 'var(--now-home)' }))} />
    </div>
    <div className="now-sum-grid">
      <div><div className="now-label">Efficiency</div><div className="now-v now-num">{efficiency.formatValue(week.efficiencyWhMi)} <small>{efficiency.label}</small></div></div>
      <div><div className="now-label">Charged</div><div className="now-v now-num">{n0(week.chargedKwh)} <small>kWh</small></div></div>
      <div><div className="now-label">Cost</div><div className="now-v now-num">{usd(week.cost)}{week.unpriced ? <> <small>+{week.unpriced}</small></> : null}</div></div>
    </div>
    {week.unpriced > 0 && <p className="now-note">{week.unpriced} {week.unpriced === 1 ? 'charge this week has' : 'charges this week have'} no cost yet.</p>}
  </section>;
}

/** Last 30 days: charging cost by place, efficiency per week and loss while parked per day. Each opens its Insights tab. */
export function NowMonth({ month }: { month: MonthSummary }) {
  const efficiency = useEfficiencyUnit();
  const totalKwh = month.byCategory.reduce((sum, item) => sum + item.kwh, 0) || 1;
  const weekValues = month.weeks.map(week => efficiency.convert(week.whMi)!).filter(Number.isFinite);
  const drainMax = Math.max(1, ...month.drainDays.map(day => day.lossPct));
  return <>
    <h2 className="now-h2">Last 30 days</h2>
    <div className="now-more3">
      <Link className="now-card now-more now-rise" to="/insights" search={{ tab: 'cost' } as never}>
        <div className="now-label">Charging cost</div>
        <div className="now-mv now-num">{usd(month.cost)}</div>
        <div className="now-stack" role="img" aria-label={`Energy charged by place: ${month.byCategory.map(item => `${item.label} ${n0(item.kwh)} kWh`).join(', ')}`}>
          {month.byCategory.filter(item => item.kwh > 0).map(item =>
            <div key={item.key} style={{ width: `${(item.kwh / totalKwh) * 100}%`, background: CATEGORY_COLOR[item.key] }} />)}
        </div>
        <div className="now-mini">{month.byCategory.map(item => <span key={item.key}><i style={{ background: CATEGORY_COLOR[item.key] }} />{item.label}{' '}
          <b className="now-num">{item.unpriced ? `${item.unpriced} not set` : item.kwh === 0 ? '–' : item.cost === 0 ? 'Free' : usd(item.cost)}</b></span>)}</div>
      </Link>
      <Link className="now-card now-more now-rise" style={{ '--i': 1 } as React.CSSProperties} to="/insights" search={{ tab: 'efficiency' } as never}>
        <div className="now-label">Efficiency</div>
        <div className="now-mv now-num">{efficiency.formatValue(month.efficiencyWhMi)} <small>{efficiency.label}</small></div>
        {weekValues.length > 0 ? <NowLineChart height={64} label="Efficiency per week" color="var(--now-home)" dots
          yDomain={paddedDomain(weekValues)} xDomain={[0, Math.max(1, weekValues.length - 1)]}
          points={month.weeks.map((week, index) => ({ x: index, y: efficiency.convert(week.whMi)!,
            title: `Week of ${formatAppDate(week.from, { year: undefined })}: ${efficiency.formatWithUnit(week.whMi)}` }))} />
          : <div className="now-chart-empty" />}
        <div className="now-mini"><span>Per week · {efficiency.lowerIsBetter ? 'lower' : 'higher'} is better</span></div>
      </Link>
      <Link className="now-card now-more now-rise" style={{ '--i': 2 } as React.CSSProperties} to="/insights" search={{ tab: 'battery' } as never}>
        <div className="now-label">Loss while parked</div>
        <div className="now-mv now-num">{month.drainPerDay == null ? '–' : n2(month.drainPerDay)} <small>% / day</small></div>
        <NowBarChart height={64} max={drainMax} label="Loss while parked per day"
          bars={month.drainDays.map(day => ({ key: day.key, value: day.lossPct, color: 'var(--now-acc)',
            title: `${day.title}: ${day.lossPct > 0 ? `${n1(day.lossPct)}% over ${n0(day.hours)} h parked` : 'No long park'}` }))} />
        <div className="now-mini"><span>Each bar is one day</span></div>
      </Link>
    </div>
  </>;
}
