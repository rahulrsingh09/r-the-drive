import { numericInput } from '../lib/controlEvents';
import * as React from 'react';
import { cn } from '../lib/utils';
import { SelectPicker } from '../primitives/SelectPicker';

export interface TableControlsProps {
  search: string;
  onSearchChange: (value: string) => void;
  searchPlaceholder: string;
  rowsPerPage: number;
  rowsPerPageOptions: readonly number[];
  onRowsPerPageChange: (value: number) => void;
  page: number;
  totalPages: number;
  totalItems: number;
  itemLabel: string;
  searchLabel?: string;
  className?: string;
}

export function TableControls({
  search,
  onSearchChange,
  searchPlaceholder,
  rowsPerPage,
  rowsPerPageOptions,
  onRowsPerPageChange,
  page,
  totalPages,
  totalItems,
  itemLabel,
  searchLabel,
  className,
}: TableControlsProps) {
  const labels = [itemLabel, `${itemLabel}s`];
  const itemLabelPlural = labels[Number(totalItems !== 1)];
  const pageOptions = Array.from(rowsPerPageOptions, value => { const text = String(value); return { value: text, label: text }; });
  const pageCount = Math.max(totalPages, 1);

  return (
    <div className={cn('flex shrink-0 flex-wrap items-center justify-between gap-3', className)}>
      <label className="relative w-full min-w-0 sm:flex-1 sm:min-w-[14rem] sm:max-w-md">
        <span className="sr-only">{searchLabel ?? `Search ${itemLabelPlural}`}</span>
        <input
          type="search"
          value={search}
          onChange={(event) => onSearchChange(event.target.value)}
          placeholder={searchPlaceholder}
          className="min-h-11 w-full rounded-lg border border-border bg-bg-surface px-3 py-2 text-sm text-fg outline-none placeholder:text-fg-tertiary focus-visible:border-accent"
        />
      </label>

      <div className="flex w-full flex-wrap items-center justify-between gap-3 sm:w-auto">
        <label className="flex items-center gap-2 text-xs text-fg-tertiary shrink-0">
          Rows
          <SelectPicker
            className="min-w-[4.5rem]"
            value={String(rowsPerPage)}
            onChange={numericInput(onRowsPerPageChange)}
            aria-label="Rows per page"
            size="sm"
            options={pageOptions}
          />
        </label>

        <p className="text-xs text-fg-tertiary">
          Page {page} of {pageCount} | {totalItems} {itemLabelPlural}
        </p>
      </div>
    </div>
  );
}
