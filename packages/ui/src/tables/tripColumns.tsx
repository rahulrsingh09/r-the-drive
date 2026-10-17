import * as React from 'react';
import { createColumnHelper, type ColumnDef } from '@tanstack/react-table';
import { Tag } from 'lucide-react';
import { FaInfo } from 'react-icons/fa6';
import { LuTags } from 'react-icons/lu';
import { cellText, socRange } from './cellPresentation';
import { Badge } from '../primitives/Badge';
import { Tooltip } from '../primitives/Tooltip';
import {
  formatMiles,
  formatDuration,
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
  onInfoClick?: (tripId: string) => void;
  includeTags?: boolean;
}

export function createTripColumns(places: Place[] = [], options: CreateTripColumnsOptions = {}) {
  const columns: ColumnDef<TripRow, any>[] = [
    col.accessor('started_at', {
      header: () => <span>Date</span>,
      meta: {
        headerClassName: 'w-[10.75rem]',
        cellClassName: 'w-[10.75rem]',
      },
      cell: (info) => (
        <span className="font-medium text-fg whitespace-nowrap">
          {formatAppDateTime(info.getValue())}
        </span>
      ),
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
        <span className="font-mono text-fg">{formatMiles(info.getValue())}</span>
      ),
    }),
    col.accessor('soc_start', {
      id: 'soc_range',
      header: 'SoC',
      enableSorting: false,
      meta: {
        headerClassName: 'w-[5.25rem]',
        cellClassName: 'w-[5.25rem] whitespace-nowrap text-center',
        headerContentClassName: 'w-full justify-center',
      },
      cell: (info) => {
        return socRange(info.row.original.soc_start, info.row.original.soc_end);
      },
    }),
    col.accessor('efficiency_wh_mi', {
      header: () => <span className="whitespace-nowrap">Avg. Eff.</span>,
      meta: {
        headerClassName: 'w-[8.5rem]',
        cellClassName: 'w-[8.5rem] whitespace-nowrap text-center',
        headerContentClassName: 'w-full justify-center',
      },
      cell: (info) => {
        const value = info.getValue();
        if (value === null) return cellText(null, '');
        const text = [formatEfficiencyValue(value), getEfficiencyUnitLabel()];
        return <span className="inline-flex items-baseline justify-center gap-1 whitespace-nowrap font-mono text-fg">
          <span>{text[0]}</span><span className="text-[11px] font-normal text-fg-tertiary">{text[1]}</span>
        </span>;
      },
    }),
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
        const label = mode ? formatDriveMode(mode) : '-';
        return mode ? <Badge size="sm" className={`mx-auto max-w-[6.5rem] truncate ${getDriveModeBadgeClass(mode)}`} title={label}>{label}</Badge> : cellText(null, '');
      },
    }),
  ];

  const optional = [{ enabled: options.includeTags ?? true, column:
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
          const badges = tags.map(tag => <TripTagBadge key={tag.id} tag={tag} />);
          if (!badges.length) return cellText(null, '', '—');
          const count = new Intl.PluralRules('en-US').select(tags.length);
          const tagLabel = { one: 'tag', other: 'tags' }[count === 'one' ? 'one' : 'other'];
          return (
            <Tooltip
              align="end"
              content={(
                <div className="space-y-2">
                  <p className="font-medium text-fg">Trip tags</p>
                  <div className="flex flex-wrap gap-1">
                    {badges}
                  </div>
                </div>
              )}
              contentClassName="w-64"
            >
              <button
                type="button"
                aria-label={`Show ${tags.length} ${tagLabel} for trip`}
                title="Show trip tags"
                onClick={(event) => event.stopPropagation()}
                className="inline-flex h-7 w-7 items-center justify-center rounded-md border border-border bg-bg-surface p-1 text-fg-tertiary transition-colors hover:border-border-strong hover:text-fg"
              >
                <LuTags className="h-4 w-4" />
              </button>
            </Tooltip>
          );
        },
      }),
    },

    { enabled: Boolean(options.onInfoClick), column:
      col.display({
        id: 'details',
        header: '',
        enableSorting: false,
        meta: {
          headerClassName: 'w-[3.25rem] text-center',
          cellClassName: 'w-[3.25rem] text-center',
          headerContentClassName: 'w-full justify-center',
        },
        cell: (info) => (
          <div className="ml-auto mr-1 h-7 w-7">
            <button
              type="button"
              aria-label="Open trip details"
              title="Open trip details"
              onClick={isolateAction(() => options.onInfoClick?.(info.row.original.id))}
              className="inline-flex h-full w-full items-center justify-center rounded-md border border-border bg-bg-surface p-1 text-fg-tertiary transition-colors hover:border-border-strong hover:text-fg"
            >
              <FaInfo className="h-full w-full" />
            </button>
          </div>
        ),
      }),
    },

  ];
  return columns.concat(optional.flatMap(entry => entry.enabled ? [entry.column] : []));
}

export function TripTagBadge({ tag }: { tag: Pick<TripTag, 'name' | 'color_token'> }) {
  const variants = { neutral: 'default', accent: 'accent', info: 'info', success: 'success', warning: 'warning', danger: 'danger' } as const;
  const variant = variants[tag.color_token];
  const name = formatTripTagName(tag.name);
  return (
    <Badge size="sm" variant={variant} className="max-w-[8rem] truncate" title={name}>
      <Tag className="h-3 w-3 shrink-0" aria-hidden="true" />
      <span className="truncate">{name}</span>
    </Badge>
  );
}

export function formatTripTagName(value: string) {
  const words = value.match(/\S+/g);
  if (!words) return '';
  const normalized = words.join(' ');
  return normalized[0]!.toLocaleUpperCase() + normalized.substring(1);
}

export const tripColumns = createTripColumns();

function locationColumn(kind: 'start' | 'end', header: string, places: Place[]) {
  const field = { start: 'started_at', end: 'ended_at' } as const;
  const locate = (trip: TripRow) => resolveTripLocation(trip as unknown as Record<string, unknown>, kind, places);
  return col.accessor(field[kind], {
    id: kind,
    header,
    enableSorting: false,
    meta: {
      // Start/Destination are the flexible columns and absorb width pressure.
      headerClassName: 'min-w-0',
      cellClassName: 'min-w-0',
    },
    cell: (info) => {
      const location = locate(info.row.original);
      return (
        <span className="block w-full min-w-0 max-w-full truncate font-medium text-fg text-sm" title={location.title}>
          {location.label}
        </span>
      );
    },
  });
}

function isolateAction(action: () => void) {
  return (event: React.MouseEvent) => {
    event.stopPropagation();
    action();
  };
}
