import * as React from 'react';
import { createColumnHelper } from '@tanstack/react-table';
import { cellText } from './cellPresentation';
import { Badge } from '../primitives/Badge';
import { formatKwh, formatDuration, formatCurrency, formatPercent } from '../lib/utils';
import { chargeSample, chargeTimes, chargeType } from './chargePresentation';

export interface ChargeSessionRow {
  id: string;
  started_at: string;
  ended_at?: string | null;
  session_day_local?: string | null;
  duration_min: number | null;
  energy_added_kwh: number | null;
  soc_start: number | null;
  soc_end: number | null;
  peak_power_kw: number | null;
  cost_usd: number | null;
  cost_method?: string | null;
  charger_type: string | null;
  location_name: string | null;
  // Enrichment fields from Rivian API
  network_vendor: string | null;
  range_added_km: number | null;
  is_free_session: boolean | null;
  is_rivian_network: boolean | null;
  rivian_paid_total: number | null;
  rivian_charger_type?: string | null;
  currency_code?: string | null;
  rivian_city?: string | null;
  is_public?: boolean | null;
  charger_id?: string | null;
  live_current_price?: number | null;
  live_current_currency?: string | null;
  live_total_charged_kwh?: number | null;
  live_range_added_km?: number | null;
  live_power_kw?: number | null;
  live_charge_rate_kph?: number | null;
  live_soc_pct?: number | null;
  live_time_elapsed_seconds?: number | null;
  live_time_remaining_min?: number | null;
  live_charger_state?: string | null;
  live_charger_status?: string | null;
  live_started_at?: string | null;
  source?: string | null;
  telemetry_sample_count?: number;
}

const col = createColumnHelper<ChargeSessionRow>();

const CHARGER_VARIANT: Record<string, 'accent' | 'info' | 'success' | 'warning'> = {
  dcfc: 'warning', dc: 'warning', ac: 'success', ac_l2: 'success',
};

export const chargingColumns = [
  col.accessor('started_at', {
    header: 'Date / Time',
    cell: (info) => {
      const times = chargeTimes(info.row.original);
      return (
        <div className="flex flex-col gap-px">
          <span className="text-sm font-medium text-fg leading-tight">{times.day}</span>
          <span className="text-xs text-fg-tertiary leading-tight">
            {times.start}
            {times.end ? ` – ${times.end}` : null}
          </span>
        </div>
      );
    },
  }),
  col.accessor('location_name', {
    header: 'Location',
    enableSorting: false,
    cell: (info) => (
      <span className="text-fg-secondary truncate max-w-[160px] block">
        {info.getValue() ?? '—'}
      </span>
    ),
  }),
  col.accessor('network_vendor', {
    header: 'Network',
    enableSorting: false,
    cell: (info) => {
      const vendor = info.getValue();
      const free = info.row.original.is_free_session && <Badge variant="success" size="sm">Free</Badge>;
      return vendor ? <span className="flex items-center gap-1.5"><span className="text-fg-secondary text-sm">{vendor}</span>{free}</span> : cellText(null, '', '—');
    },
  }),
  col.accessor('charger_type', {
    header: 'Type',
    enableSorting: false,
    cell: (info) => {
      const row = info.row.original;
      const inferred = chargeType(row);
      if (!inferred) return <span className="text-fg-tertiary">—</span>;
      return (
        <Badge variant={CHARGER_VARIANT[inferred] ?? 'default'} size="sm">
          {inferred.toUpperCase()}
        </Badge>
      );
    },
  }),
  col.accessor('energy_added_kwh', {
    header: 'Energy Added',
    cell: (info) => {
      const row = info.row.original;
      const { value: v, suffix } = chargeSample(row, 'energy');
      return v !== null ? (
        <span className="font-mono">{formatKwh(v)}{suffix}</span>
      ) : <span className="text-fg-tertiary">—</span>;
    },
  }),
  col.accessor('soc_start', {
    header: 'SoC',
    enableSorting: false,
    cell: (info) => {
      const row = info.row.original;
      const start = row.soc_start;
      const { value: end, suffix } = chargeSample(row, 'soc');
      if (start === null || end === null) return <span className="text-fg-tertiary">—</span>;
      return (
        <span className="font-mono text-fg">
          {formatPercent(start, 0)} → {formatPercent(end, 0)}{suffix}
        </span>
      );
    },
  }),
  col.accessor('peak_power_kw', {
    header: 'Peak',
    cell: (info) => {
      const row = info.row.original;
      const { value: v } = chargeSample(row, 'power');
      return v !== null ? (
        <span className="font-mono">{v.toFixed(1)} kW</span>
      ) : <span className="text-fg-tertiary">—</span>;
    },
  }),
  col.accessor('duration_min', {
    header: 'Duration',
    cell: (info) => {
      const row = info.row.original;
      const { value: v, suffix } = chargeSample(row, 'duration');
      return v !== null ? <>{formatDuration(v)}{suffix}</> : <span className="text-fg-tertiary">—</span>;
    },
  }),
  col.accessor('cost_usd', {
    header: 'Cost',
    cell: (info) => {
      const value = info.getValue();
      const missing = info.row.original.ended_at == null ? 'Pending' : '—';
      return cellText(value === null ? null : formatCurrency(value), 'font-mono text-accent', missing);
    },
  }),
];
