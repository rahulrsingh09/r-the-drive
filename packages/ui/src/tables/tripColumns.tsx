import * as React from 'react';
import { createColumnHelper, type ColumnDef } from '@tanstack/react-table';
import { Check, Tag } from 'lucide-react';
import { LuTags } from 'react-icons/lu';
import { Badge } from '../primitives/Badge';
import { Tooltip } from '../primitives/Tooltip';
import {
  cn,
  formatMiles,
  formatDuration,
  formatPercent,
  formatEfficiencyValue,
  getEfficiencyUnitLabel,
} from '../lib/utils';
import { formatDriveMode, getDriveModeBadgeClass } from '../lib/driveMode';
import { resolveTripLocation } from '../lib/tripPresentation';
import { formatAppDateTime } from '../lib/dateTime';
import type { Place, TripTag } from '@panda/types';

export interface TripRow {
  id: string;
  started_at: string;
  ended_at: string | null;
  distance_mi: number;
  duration_min: number;
  energy_used_kwh: number | null;
  efficiency_wh_mi: number | null;
  soc_start: number | null;
  soc_end: number | null;
  drive_mode?: string | null;
  start_lat?: number | null;
  start_lng?: number | null;
  end_lat?: number | null;
  end_lng?: number | null;
  // Reverse-geocoded labels returned by the API
  start_address?: string | null;
  end_address?: string | null;
  start_place?: string | null;
  end_place?: string | null;
  tags?: TripTag[];
}

const col = createColumnHelper<TripRow>();

interface CreateTripColumnsOptions {
  /** Opens the trip. The row also opens it; this makes the date cell a keyboard target. */
  onOpenTrip?: (tripId: string) => void;
  /** Adds a leading checkbox that selects the trip for the map and for tag edits. */
  onToggleSelect?: (tripId: string) => void;
  isSelected?: (tripId: string) => boolean;
  includeTags?: boolean;
  /** Set false when every visible trip uses the default drive mode. */
  includeMode?: boolean;
}

/** All-Purpose is the default drive mode. The list shows a mode chip only for other modes. */
export function isDefaultTripDriveMode(mode: string | null | undefined) {
  return !mode || formatDriveMode(mode) === 'All-Purpose';
}

/** Battery at the start and end of a trip, for example "75% → 68%". */
export function formatTripBatteryRange(start: number | null | undefined, end: number | null | undefined) {
  if (start == null || end == null) return null;
  return `${formatPercent(start)} → ${formatPercent(end)}`;
}

export interface TripSelectCheckboxProps {
  checked: boolean;
  onChange: () => void;
  label: string;
  className?: string;
}

/**
 * Trip selection checkbox with a 44 × 44 px touch target. The input fills the
 * whole target and stays visible to assistive technology; the 16 px box is
 * drawn by the sibling elements so the table keeps its compact look. Negative
 * margins keep the layout footprint at 28 px.
 */
export function TripSelectCheckbox({ checked, onChange, label, className }: TripSelectCheckboxProps) {
  return (
    <label
      className={cn('relative -m-2 inline-flex h-11 w-11 shrink-0 cursor-pointer items-center justify-center', className)}
      onClick={(event) => event.stopPropagation()}
    >
      <input
        type="checkbox"
        className="peer absolute inset-0 m-0 h-full w-full cursor-pointer appearance-none rounded-md bg-transparent focus-visible:outline-none"
        checked={checked}
        onChange={onChange}
        aria-label={label}
      />
      <span
        aria-hidden="true"
        className="pointer-events-none h-4 w-4 rounded border border-border-strong bg-bg-surface transition-colors peer-checked:border-accent peer-checked:bg-accent peer-focus-visible:ring-2 peer-focus-visible:ring-accent peer-focus-visible:ring-offset-2 peer-focus-visible:ring-offset-bg-page"
      />
      <Check
        aria-hidden="true"
        strokeWidth={3}
        className="pointer-events-none absolute left-1/2 top-1/2 h-3 w-3 -translate-x-1/2 -translate-y-1/2 text-fg-on-accent opacity-0 peer-checked:opacity-100"
      />
    </label>
  );
}

export function createTripColumns(places: Place[] = [], options: CreateTripColumnsOptions = {}) {
  const includeTags = options.includeTags ?? true;
  const includeMode = options.includeMode ?? true;
  const { onOpenTrip, onToggleSelect, isSelected } = options;
  const columns: ColumnDef<TripRow, any>[] = [];
  if (onToggleSelect) {
    columns.push(col.display({
      id: 'select',
      header: () => <span className="sr-only">Select</span>,
      enableSorting: false,
      meta: {
        headerClassName: 'w-[3rem] text-center',
        cellClassName: 'w-[3rem] text-center',
        headerContentClassName: 'w-full justify-center',
      },
      cell: (info) => {
        const trip = info.row.original;
        return (
          <TripSelectCheckbox
            checked={isSelected?.(trip.id) ?? false}
            onChange={() => onToggleSelect(trip.id)}
            label={`Select trip from ${formatAppDateTime(trip.started_at)}`}
          />
        );
      },
    }));
  }
  columns.push(
    col.accessor('started_at', {
      header: () => <span>Date</span>,
      meta: {
        headerClassName: 'w-[10.75rem]',
        cellClassName: 'w-[10.75rem]',
      },
      cell: (info) => {
        const label = formatAppDateTime(info.getValue());
        if (!onOpenTrip) return <span className="font-medium text-fg whitespace-nowrap">{label}</span>;
        return (
          <button
            type="button"
            className="relative whitespace-nowrap rounded-sm text-left font-medium text-fg before:absolute before:inset-x-0 before:top-1/2 before:h-11 before:-translate-y-1/2 before:content-[''] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
            aria-label={`Open trip from ${label}`}
            onClick={(event) => {
              event.stopPropagation();
              onOpenTrip(info.row.original.id);
            }}
          >
            {label}
          </button>
        );
      },
    }),
    locationColumn('start', 'Start', places),
    locationColumn('end', 'Destination', places),
    col.accessor('duration_min', {
      header: 'Duration',
      meta: {
        headerClassName: 'w-[5.25rem]',
        cellClassName: 'w-[5.25rem] whitespace-nowrap text-center',
        headerContentClassName: 'w-full justify-center',
      },
      cell: (info) => formatDuration(info.getValue()),
    }),
    col.accessor('distance_mi', {
      header: 'Distance',
      meta: {
        headerClassName: 'w-[5.25rem]',
        cellClassName: 'w-[5.25rem] whitespace-nowrap text-center',
        headerContentClassName: 'w-full justify-center',
      },
      cell: (info) => (
        <span className="tabular-nums text-fg">{formatMiles(info.getValue())}</span>
      ),
    }),
    col.accessor('soc_start', {
      id: 'soc_range',
      header: 'Battery',
      enableSorting: false,
      meta: {
        headerClassName: 'w-[6.75rem]',
        cellClassName: 'w-[6.75rem] whitespace-nowrap text-center',
        headerContentClassName: 'w-full justify-center',
      },
      cell: (info) => {
        const row = info.row.original;
        const range = formatTripBatteryRange(row.soc_start, row.soc_end);
        if (!range) return <span className="text-fg-tertiary">-</span>;
        return <span className="whitespace-nowrap tabular-nums text-fg">{range}</span>;
      },
    }),
    col.accessor('efficiency_wh_mi', {
      header: () => <span className="whitespace-nowrap">Efficiency</span>,
      meta: {
        headerClassName: 'w-[8.5rem]',
        cellClassName: 'w-[8.5rem] whitespace-nowrap text-center',
        headerContentClassName: 'w-full justify-center',
      },
      cell: (info) => {
        const v = info.getValue();
        return v !== null ? (
          <span className="inline-flex items-baseline justify-center gap-1 whitespace-nowrap tabular-nums text-fg">
            <span>{formatEfficiencyValue(v)}</span>
            <span className="text-[11px] font-normal text-fg-tertiary">{getEfficiencyUnitLabel()}</span>
          </span>
        ) : (
          <span className="text-fg-tertiary">-</span>
        );
      },
    }),
  );

  if (includeMode) {
    columns.push(
      col.accessor('drive_mode', {
        header: 'Mode',
        enableSorting: false,
        meta: {
          headerClassName: 'w-[6.5rem]',
          cellClassName: 'w-[6.5rem] text-center',
          headerContentClassName: 'w-full justify-center',
        },
        cell: (info) => {
          const mode = info.getValue();
          if (!mode || isDefaultTripDriveMode(mode)) return null;
          return (
            <Badge size="sm" className={`mx-auto max-w-[6.5rem] truncate ${getDriveModeBadgeClass(mode)}`} title={formatDriveMode(mode)}>
              {formatDriveMode(mode)}
            </Badge>
          );
        },
      }),
    );
  }

  if (includeTags) {
    columns.push(
      col.accessor('tags', {
        header: 'Tags',
        enableSorting: false,
        meta: {
          headerClassName: 'w-[3.25rem] text-center',
          cellClassName: 'w-[3.25rem] text-center',
          headerContentClassName: 'w-full justify-center',
        },
        cell: (info) => {
          const tags: TripTag[] = info.getValue() ?? [];
          if (tags.length === 0) return <span className="text-fg-tertiary">—</span>;
          return (
            <Tooltip
              align="end"
              content={(
                <div className="space-y-2">
                  <p className="font-medium text-fg">Trip tags</p>
                  <div className="flex flex-wrap gap-1">
                    {tags.map((tag) => <TripTagBadge key={tag.id} tag={tag} />)}
                  </div>
                </div>
              )}
              contentClassName="w-64"
            >
              <button
                type="button"
                aria-label={`Show ${tags.length} tag${tags.length === 1 ? '' : 's'} for trip`}
                title="Show trip tags"
                onClick={(event) => event.stopPropagation()}
                className="relative inline-flex h-7 w-7 items-center justify-center rounded-md border border-border bg-bg-surface p-1 text-fg-tertiary transition-colors before:absolute before:left-1/2 before:top-1/2 before:h-11 before:w-11 before:-translate-x-1/2 before:-translate-y-1/2 before:content-[''] hover:border-border-strong hover:text-fg"
              >
                <LuTags className="h-4 w-4" />
              </button>
            </Tooltip>
          );
        },
      }),
    );
  }

  return columns;
}

export function TripTagBadge({ tag }: { tag: Pick<TripTag, 'name' | 'color_token'> }) {
  const variant = tag.color_token === 'neutral' ? 'default' : tag.color_token;
  const name = formatTripTagName(tag.name);
  return (
    <Badge size="sm" variant={variant} className="max-w-[8rem] truncate" title={name}>
      <Tag className="h-3 w-3 shrink-0" aria-hidden="true" />
      <span className="truncate">{name}</span>
    </Badge>
  );
}

export function formatTripTagName(value: string) {
  const normalized = value.trim().replace(/\s+/g, ' ');
  return normalized ? `${normalized.charAt(0).toLocaleUpperCase()}${normalized.slice(1)}` : normalized;
}

export const tripColumns = createTripColumns();

function locationColumn(kind: 'start' | 'end', header: string, places: Place[]) {
  return col.accessor(kind === 'start' ? 'started_at' : 'ended_at', {
    id: kind,
    header,
    enableSorting: false,
    meta: {
      // Start/Destination are the flexible columns and absorb width pressure.
      headerClassName: 'min-w-0',
      cellClassName: 'min-w-0',
    },
    cell: (info) => {
      const trip = info.row.original as unknown as Record<string, unknown>;
      const location = resolveTripLocation(trip, kind, places);
      return (
        <span className="block w-full min-w-0 max-w-full truncate font-medium text-fg text-sm" title={location.title}>
          {location.label}
        </span>
      );
    },
  });
}
