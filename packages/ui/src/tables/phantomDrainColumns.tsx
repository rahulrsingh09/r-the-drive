import * as React from 'react';
import { createColumnHelper, type CellContext } from '@tanstack/react-table';
import { PiArrowFatLinesRight } from 'react-icons/pi';
import { Tooltip } from '../primitives/Tooltip';
import { formatKwh, formatMiles, formatPercent, formatSmartNumber } from '../lib/utils';
import { formatAppDateTime } from '../lib/dateTime';

export interface PhantomDrainRow {
  period_start: string | null;
  period_end: string | null;
  duration_hours: number | null;
  sleep_share_pct: number | null;
  state_coverage_pct: number | null;
  soc_start: number | null;
  soc_end: number | null;
  soc_lost_pct: number | null;
  drain_pct_per_hour: number | null;
  range_start_mi: number | null;
  range_end_mi: number | null;
  range_lost_mi: number | null;
  range_lost_per_hour_mi: number | null;
  energy_drained_kwh: number | null;
  avg_power_w: number | null;
  has_reduced_range: boolean | null;
  validation_status: 'validated' | 'excluded';
  validation_reason: string | null;
  sample_count: number;
  start_sample_at: string | null;
  end_sample_at: string | null;
  movement_detected: boolean;
  overlaps_trip: boolean;
  overlaps_charge: boolean;
}

const col = createColumnHelper<PhantomDrainRow>();

function formatDateTime(value: string | null) {
  if (!value) return '-';
  return formatAppDateTime(value);
}

function drainNumber(value: number | null, decimals: number, suffix: string, factor = 1) {
  return value == null || Number.isNaN(value) ? '-' : `${formatSmartNumber(value / factor, decimals)}${suffix}`;
}
export function formatDurationHours(value: number | null) {
  return drainNumber(value, 1, ' h');
}
export function formatPowerWatts(value: number | null) {
  const kilo = value != null && Math.abs(value) >= 1000;
  return drainNumber(value, kilo ? 2 : 0, kilo ? ' kW' : ' W', kilo ? 1000 : 1);
}
export function formatRatioPercent(value: number | null, decimals = 0) {
  return value == null || Number.isNaN(value) ? '-' : formatPercent(value * 100, decimals);
}
const DRAIN_METRICS = {
  sleep_share_pct: { empty: 'Unknown', className: 'font-mono text-fg-secondary', format: (value: number) => formatRatioPercent(value, 0) },
  state_coverage_pct: { empty: '-', className: 'font-mono text-fg-secondary', format: (value: number) => formatRatioPercent(value, 0) },
  soc_lost_pct: { empty: '-', className: 'font-mono text-fg', format: (value: number) => `-${formatPercent(value, 2)}` },
  drain_pct_per_hour: { empty: '-', className: 'font-mono text-fg', format: (value: number) => `${formatPercent(value, 2)} / h` },
  range_lost_mi: { empty: '-', className: 'font-mono text-fg', format: formatMiles },
  range_lost_per_hour_mi: { empty: '-', className: 'font-mono text-fg-secondary', format: (value: number) => `${formatMiles(value, 2)} / h` },
} as const;
function metricCell(field: keyof typeof DRAIN_METRICS) {
  const contract = DRAIN_METRICS[field];
  return (info: CellContext<PhantomDrainRow, number | null>) => {
    const value = info.getValue();
    const missing = value == null || Number.isNaN(value);
    return <span className={missing ? 'text-fg-tertiary' : contract.className}>{missing ? contract.empty : contract.format(value)}</span>;
  };
}

function infoHeader(label: string, description: string) {
  return (
    <span className="inline-flex items-center gap-1">
      <span>{label}</span>
      <Tooltip content={description}>
        <span
          aria-label={`${label} info`}
          className="inline-flex h-4 w-4 items-center justify-center rounded-full border border-border text-[10px] text-fg-tertiary"
        >
          i
        </span>
      </Tooltip>
    </span>
  );
}

function reasonLabel(value: string | null) {
  if (!value) return '-';
  return value.replaceAll('_', ' ');
}

export const phantomDrainColumns = [
  col.accessor('period_start', {
    header: 'Start',
    meta: {
      headerClassName: 'w-[9.25rem]',
      cellClassName: 'w-[9.25rem]',
      columnLabel: 'Start',
    },
    cell: (info) => {
      const value = info.getValue();
      return (
        <span className="whitespace-nowrap text-fg" title={value ? formatAppDateTime(value) : undefined}>
          {formatDateTime(value)}
        </span>
      );
    },
  }),
  col.accessor('period_end', {
    header: 'End',
    meta: {
      headerClassName: 'w-[9.25rem]',
      cellClassName: 'w-[9.25rem]',
      columnLabel: 'End',
    },
    cell: (info) => {
      const value = info.getValue();
      return (
        <span className="whitespace-nowrap text-fg" title={value ? formatAppDateTime(value) : undefined}>
          {formatDateTime(value)}
        </span>
      );
    },
  }),
  col.accessor('duration_hours', {
    header: 'Dur',
    meta: {
      headerClassName: 'w-[4.75rem] text-center',
      cellClassName: 'w-[4.75rem] whitespace-nowrap text-center',
      headerContentClassName: 'w-full justify-center',
      columnLabel: 'Duration',
    },
    cell: (info) => <span className="font-mono text-fg-secondary">{formatDurationHours(info.getValue())}</span>,
  }),
  col.accessor('sleep_share_pct', {
    header: () => infoHeader(
      'Sleep',
      'Share of this validated parked session covered by sleep state. Shows a dash when state coverage is too incomplete to trust.',
    ),
    meta: {
      headerClassName: 'w-[5.25rem] text-center',
      cellClassName: 'w-[5.25rem] whitespace-nowrap text-center',
      headerContentClassName: 'w-full justify-center',
      columnLabel: 'Sleep',
    },
    cell: metricCell('sleep_share_pct'),
  }),
  col.accessor('state_coverage_pct', {
    header: () => infoHeader(
      'State Cov',
      'How much of this session has usable vehicle-state coverage. Low coverage means sleep share is hidden instead of guessed.',
    ),
    meta: {
      headerClassName: 'w-[5.75rem] text-center',
      cellClassName: 'w-[5.75rem] whitespace-nowrap text-center',
      headerContentClassName: 'w-full justify-center',
      columnLabel: 'State Coverage',
    },
    cell: metricCell('state_coverage_pct'),
  }),
  col.accessor('soc_start', {
    id: 'soc_range',
    header: 'SoC',
    enableSorting: false,
    meta: {
      headerClassName: 'w-[7.75rem] text-center',
      cellClassName: 'w-[7.75rem] whitespace-nowrap text-center',
      headerContentClassName: 'w-full justify-center',
      columnLabel: 'SoC',
    },
    cell: (info) => {
      const row = info.row.original;
      if (row.soc_start == null || row.soc_end == null) return <span className="text-fg-tertiary">-</span>;
      return (
        <span className="inline-flex items-center gap-1 whitespace-nowrap font-mono text-fg">
          {formatPercent(row.soc_start, 0)}
          <PiArrowFatLinesRight className="h-3.5 w-3.5 shrink-0 text-fg-tertiary" />
          {formatPercent(row.soc_end, 0)}
        </span>
      );
    },
  }),
  col.accessor('soc_lost_pct', {
    header: () => infoHeader('SoC Diff', 'Battery percentage dropped during this validated parked session.'),
    meta: {
      headerClassName: 'w-[5.25rem] text-center',
      cellClassName: 'w-[5.25rem] whitespace-nowrap text-center',
      headerContentClassName: 'w-full justify-center',
      columnLabel: 'SoC Diff',
    },
    cell: metricCell('soc_lost_pct'),
  }),
  col.accessor('drain_pct_per_hour', {
    header: () => infoHeader('Drain / h', 'Validated battery percentage lost per parked hour during this session.'),
    meta: {
      headerClassName: 'w-[5.75rem] text-center',
      cellClassName: 'w-[5.75rem] whitespace-nowrap text-center',
      headerContentClassName: 'w-full justify-center',
      columnLabel: 'Drain / h',
    },
    cell: metricCell('drain_pct_per_hour'),
  }),
  col.accessor('range_lost_mi', {
    header: () => infoHeader(
      'Range',
      'Validated range loss from boundary samples. Values are hidden when the range anchors do not agree with the SoC-backed drain math.',
    ),
    meta: {
      headerClassName: 'w-[5.75rem] text-center',
      cellClassName: 'w-[5.75rem] whitespace-nowrap text-center',
      headerContentClassName: 'w-full justify-center',
      columnLabel: 'Range',
    },
    cell: metricCell('range_lost_mi'),
  }),
  col.accessor('range_lost_per_hour_mi', {
    header: () => infoHeader('Range / h', 'Estimated miles of range lost per hour over this validated parked session.'),
    meta: {
      headerClassName: 'w-[5.75rem] text-center',
      cellClassName: 'w-[5.75rem] whitespace-nowrap text-center',
      headerContentClassName: 'w-full justify-center',
      columnLabel: 'Range / h',
    },
    cell: metricCell('range_lost_per_hour_mi'),
  }),
  col.accessor('energy_drained_kwh', {
    header: 'kWh',
    meta: {
      headerClassName: 'w-[5.5rem] text-center',
      cellClassName: 'w-[5.5rem] whitespace-nowrap text-center',
      headerContentClassName: 'w-full justify-center',
      columnLabel: 'kWh',
    },
    cell: (info) => <span className="font-mono text-fg">{formatKwh(info.getValue())}</span>,
  }),
  col.accessor('avg_power_w', {
    header: () => infoHeader(
      'Power',
      'Average power draw inferred from validated SoC loss and estimated battery capacity during this session.',
    ),
    meta: {
      headerClassName: 'w-[5.5rem] text-center',
      cellClassName: 'w-[5.5rem] whitespace-nowrap text-center',
      headerContentClassName: 'w-full justify-center',
      columnLabel: 'Power',
    },
    cell: (info) => <span className="font-mono text-fg-secondary">{formatPowerWatts(info.getValue())}</span>,
  }),
  col.accessor('validation_reason', {
    header: 'Reason',
    meta: {
      headerClassName: 'w-[8rem]',
      cellClassName: 'w-[8rem] whitespace-nowrap',
      columnLabel: 'Reason',
    },
    cell: (info) => <span className="text-fg-secondary capitalize">{reasonLabel(info.getValue())}</span>,
  }),
  col.accessor('has_reduced_range', {
    header: ' ',
    meta: {
      headerClassName: 'w-8 text-center',
      cellClassName: 'w-8 text-center',
      headerContentClassName: 'w-full justify-center',
      columnLabel: 'Reduced range',
    },
    cell: (info) => {
      if (!info.getValue()) return <span className="text-fg-tertiary"> </span>;
      return (
        <span
          role="img"
          aria-label="Reduced range conditions"
          title="Estimated range loss may be impacted by reduced-range conditions."
          className="text-accent"
        >
          {'\u2744'}
        </span>
      );
    },
  }),
];
