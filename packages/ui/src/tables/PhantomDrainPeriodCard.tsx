import * as React from 'react';
import { ChevronDown } from 'lucide-react';
import { formatAppDate, formatAppDateTime, formatAppTime } from '../lib/dateTime';
import { formatKwh, formatMiles, formatPercent } from '../lib/utils';
import { formatDurationHours, formatPowerWatts, formatRatioPercent, type PhantomDrainRow } from './phantomDrainColumns';

export function PhantomDrainPeriodCard({ period }: { period: PhantomDrainRow }) {
  const number = (value: number | null, format: (value: number) => string, empty = '—') => value == null ? empty : format(value);
  const timestamp = (value: string | null, format: (value: string) => string, empty: string) => value ? format(value) : empty;
  const start = period.period_start;
  const heading = timestamp(start, formatAppDate, 'Unknown date');
  const when = timestamp(start, value => `${formatAppTime(value)} · `, '');
  const summary = [
    ['Battery lost', number(period.soc_lost_pct, value => formatPercent(value, 2))],
    ['Per hour', number(period.drain_pct_per_hour, value => `${formatPercent(value, 2)}/h`)],
    ['Asleep', number(period.sleep_share_pct, formatRatioPercent, 'Unknown')],
  ];
  const dates = [
    ['Parked from', timestamp(start, formatAppDateTime, 'Unknown')],
    ['Until', timestamp(period.period_end, formatAppDateTime, 'Unknown')],
  ];
  const readings = [
    ['Charge level', period.soc_start == null || period.soc_end == null ? '—' : [period.soc_start, period.soc_end].map(value => formatPercent(value, 0)).join(' → ')],
    ['Energy used', formatKwh(period.energy_drained_kwh)],
    ['Range lost', formatMiles(period.range_lost_mi)],
    ['Range per hour', number(period.range_lost_per_hour_mi, value => `${formatMiles(value, 2)}/h`)],
    ['Average power', formatPowerWatts(period.avg_power_w)],
    ['State coverage', formatRatioPercent(period.state_coverage_pct)],
  ];
  const notes = [
    { show: period.sleep_share_pct == null, className: 'mt-4 text-xs text-fg-secondary', text: 'Sleep is unknown when vehicle-state coverage is incomplete.' },
    { show: Boolean(period.validation_reason), className: 'mt-3 text-xs text-fg-secondary', text: period.validation_reason?.replaceAll('_', ' ') },
    { show: Boolean(period.has_reduced_range), className: 'mt-3 text-xs text-fg-secondary', text: 'Reduced-range conditions may affect the range estimate.' },
  ];
  return (
    <details className="group rounded-xl border border-border bg-bg-surface" data-testid="phantom-period-card">
      <summary className="cursor-pointer list-none p-4 [&::-webkit-details-marker]:hidden">
        <span className="flex items-center justify-between gap-3">
          <span className="min-w-0">
            <span className="block text-sm font-medium text-fg">{heading}</span>
            <span className="mt-1 block text-xs text-fg-secondary">
              {when}{formatDurationHours(period.duration_hours)} parked
            </span>
          </span>
          <ChevronDown aria-hidden="true" className="h-4 w-4 shrink-0 text-fg-secondary transition-transform group-open:rotate-180 motion-reduce:transition-none" />
        </span>
        <span className="mt-4 grid grid-cols-3 gap-2">
          {summary.map(([label, value]) => (
            <span key={label} className="min-w-0">
              <span className="block text-xs text-fg-secondary">{label}</span>
              <span className="mt-1 block break-words text-base font-medium tabular-nums text-fg">{value}</span>
            </span>
          ))}
        </span>
        <span className="mt-3 block text-xs text-fg-tertiary group-open:hidden">View period details</span>
      </summary>
      <div className="border-t border-border px-4 pb-4 pt-3">
        <dl className="grid gap-2 text-xs">
          {dates.map(([label, value]) => <div key={label} className="flex flex-wrap justify-between gap-x-3 gap-y-1">
            <dt className="text-fg-secondary">{label}</dt><dd className="text-fg">{value}</dd>
          </div>)}
        </dl>
        <dl className="mt-4 grid grid-cols-2 gap-x-4 gap-y-4">
          {readings.map(([label, value]) => <div key={label} className="min-w-0">
            <dt className="text-xs text-fg-secondary">{label}</dt><dd className="mt-1 break-words text-sm tabular-nums text-fg">{value}</dd>
          </div>)}
        </dl>
        {notes.filter(note => note.show).map((note, index) => <p key={index} className={note.className}>{note.text}</p>)}
      </div>
    </details>
  );
}
