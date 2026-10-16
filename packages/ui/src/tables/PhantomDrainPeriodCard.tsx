import * as React from 'react';
import { ChevronDown } from 'lucide-react';
import { formatAppDate, formatAppDateTime, formatAppTime } from '../lib/dateTime';
import { formatKwh, formatMiles, formatPercent } from '../lib/utils';
import { formatDurationHours, formatPowerWatts, formatRatioPercent, type PhantomDrainRow } from './phantomDrainColumns';

export function PhantomDrainPeriodCard({ period }: { period: PhantomDrainRow }) {
  const loss = period.soc_lost_pct == null ? '—' : formatPercent(period.soc_lost_pct, 2);
  const rate = period.drain_pct_per_hour == null ? '—' : `${formatPercent(period.drain_pct_per_hour, 2)}/h`;
  const sleep = period.sleep_share_pct == null ? 'Unknown' : formatRatioPercent(period.sleep_share_pct);
  const dates = [
    ['Parked from', period.period_start ? formatAppDateTime(period.period_start) : 'Unknown'],
    ['Until', period.period_end ? formatAppDateTime(period.period_end) : 'Unknown'],
  ];
  const readings = [
    ['Charge level', period.soc_start == null || period.soc_end == null ? '—' : `${formatPercent(period.soc_start, 0)} → ${formatPercent(period.soc_end, 0)}`],
    ['Energy used', formatKwh(period.energy_drained_kwh)],
    ['Range lost', formatMiles(period.range_lost_mi)],
    ['Range per hour', period.range_lost_per_hour_mi == null ? '—' : `${formatMiles(period.range_lost_per_hour_mi, 2)}/h`],
    ['Average power', formatPowerWatts(period.avg_power_w)],
    ['State coverage', formatRatioPercent(period.state_coverage_pct)],
  ];
  return (
    <details className="group rounded-xl border border-border bg-bg-surface" data-testid="phantom-period-card">
      <summary className="cursor-pointer list-none p-4 [&::-webkit-details-marker]:hidden">
        <span className="flex items-center justify-between gap-3">
          <span className="min-w-0">
            <span className="block text-sm font-medium text-fg">{period.period_start ? formatAppDate(period.period_start) : 'Unknown date'}</span>
            <span className="mt-1 block text-xs text-fg-secondary">
              {period.period_start ? `${formatAppTime(period.period_start)} · ` : ''}{formatDurationHours(period.duration_hours)} parked
            </span>
          </span>
          <ChevronDown aria-hidden="true" className="h-4 w-4 shrink-0 text-fg-secondary transition-transform group-open:rotate-180 motion-reduce:transition-none" />
        </span>
        <span className="mt-4 grid grid-cols-3 gap-2">
          {[['Battery lost', loss], ['Per hour', rate], ['Asleep', sleep]].map(([label, value]) => (
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
        {period.sleep_share_pct == null ? <p className="mt-4 text-xs text-fg-secondary">Sleep is unknown when vehicle-state coverage is incomplete.</p> : null}
        {period.validation_reason ? <p className="mt-3 text-xs text-fg-secondary">{period.validation_reason.replaceAll('_', ' ')}</p> : null}
        {period.has_reduced_range ? <p className="mt-3 text-xs text-fg-secondary">Reduced-range conditions may affect the range estimate.</p> : null}
      </div>
    </details>
  );
}
