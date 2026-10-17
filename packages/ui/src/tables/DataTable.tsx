import * as React from 'react';
import {
  useReactTable,
  getCoreRowModel,
  getSortedRowModel,
  flexRender,
  type ColumnDef,
  type Row,
} from '@tanstack/react-table';
import { ChevronUp, ChevronDown, ChevronsUpDown } from 'lucide-react';
import { cn } from '../lib/utils';
import { useDataTableState } from '../hooks/useDataTableState';
import { Skeleton } from '../primitives/Skeleton';
import { EmptyState } from '../primitives/EmptyState';
import { SelectPicker } from '../primitives/SelectPicker';

type ColumnMeta = {
  headerClassName?: string;
  cellClassName?: string;
  headerContentClassName?: string;
  columnLabel?: string;
};

export interface DataTableProps<TData> {
  data: TData[];
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  columns: ColumnDef<TData, any>[];
  loading?: boolean;
  loadingRows?: number;
  onRowClick?: (row: Row<TData>) => void;
  getRowIsSelected?: (row: Row<TData>) => boolean;
  emptyTitle?: string;
  emptyDescription?: string;
  className?: string;
  fixedLayout?: boolean;
  columnVisibilityMenu?: boolean;
  defaultHiddenColumns?: string[];
  renderMobileRow?: (row: Row<TData>) => React.ReactNode;
}

export function DataTable<TData>({
  data,
  columns,
  loading = false,
  loadingRows = 8,
  onRowClick,
  getRowIsSelected,
  emptyTitle = 'No data',
  emptyDescription,
  className,
  fixedLayout = false,
  columnVisibilityMenu = false,
  defaultHiddenColumns,
  renderMobileRow,
}: DataTableProps<TData>) {
  const { sorting, columnVisibility, menuPosition, setSorting, setColumnVisibility, openColumnMenu, chooseSort, reverseSort } = useDataTableState(defaultHiddenColumns, columnVisibilityMenu);

  const table = useReactTable({
    data,
    columns,
    state: { sorting, columnVisibility },
    onSortingChange: setSorting,
    onColumnVisibilityChange: setColumnVisibility,
    getCoreRowModel: getCoreRowModel(),
    getSortedRowModel: getSortedRowModel(),
  });

  const visibleColumns = table.getVisibleLeafColumns();
  const menuColumns = table.getAllLeafColumns().flatMap(column => {
    const { header, meta } = column.columnDef;
    const metadata = meta as ColumnMeta | undefined;
    const label = typeof header === 'string' ? header : metadata?.columnLabel;
    return typeof label === 'string' ? [{ column, label, sortLabel: metadata?.columnLabel ?? String(header) }] : [];
  });

  return (
    <div className={cn('relative w-full overflow-x-auto', className)}>
      {renderMobileRow ? (
        <div className="grid gap-3 py-3 md:hidden" data-testid="mobile-table-cards">
          <div className="flex items-center gap-2">
            <SelectPicker
              aria-label="Sort rows by"
              className="min-w-0 flex-1"
              value={sorting[0]?.id ?? ''}
              onChange={chooseSort}
              options={[
                { value: '', label: 'Original order' },
                ...menuColumns.filter(({ column }) => column.getCanSort()).map(({ column, sortLabel }) => ({ value: column.id, label: sortLabel })),
              ]}
            />
            <button
              type="button"
              aria-label={sorting[0]?.desc ? 'Sort ascending' : 'Sort descending'}
              disabled={!sorting.length}
              onClick={reverseSort}
              className="flex h-11 w-11 shrink-0 items-center justify-center rounded-lg border border-border bg-bg-surface text-fg-secondary transition-colors hover:bg-bg-elevated disabled:opacity-40"
            >
              {sorting[0]?.desc ? <ChevronDown aria-hidden="true" className="h-4 w-4" /> : <ChevronUp aria-hidden="true" className="h-4 w-4" />}
            </button>
          </div>
          {loading ? Array.from({ length: Math.min(loadingRows, 5) }, (_, i) => <Skeleton key={i} className="h-36" />)
            : table.getRowModel().rows.length === 0 ? <EmptyState title={emptyTitle} description={emptyDescription} />
            : table.getRowModel().rows.map((row) => <React.Fragment key={row.id}>{renderMobileRow(row)}</React.Fragment>)}
        </div>
      ) : null}
      <table className={cn('w-full text-sm', fixedLayout && 'table-fixed', renderMobileRow && 'hidden md:table')}>
        <thead onContextMenu={openColumnMenu}>
          {table.getHeaderGroups().map((headerGroup) => (
            <tr key={headerGroup.id} className="border-b border-border">
              {headerGroup.headers.map((header) => {
                const canSort = header.column.getCanSort();
                const sorted = header.column.getIsSorted();
                const meta = header.column.columnDef.meta as ColumnMeta | undefined;
                return (
                  <th
                    key={header.id}
                    className={cn(
                      'py-2 px-3 text-left text-xs font-medium text-fg-tertiary uppercase tracking-wider whitespace-nowrap',
                      canSort && 'cursor-pointer select-none hover:text-fg-secondary',
                      meta?.headerClassName
                    )}
                    onClick={canSort ? header.column.getToggleSortingHandler() : undefined}
                    onContextMenu={openColumnMenu}
                  >
                    <span className={cn('flex items-center gap-1', meta?.headerContentClassName)}>
                      {flexRender(header.column.columnDef.header, header.getContext())}
                      {canSort && (
                        <span className="text-fg-tertiary">
                          {sorted === 'asc' ? (
                            <ChevronUp className="h-3 w-3" />
                          ) : sorted === 'desc' ? (
                            <ChevronDown className="h-3 w-3" />
                          ) : (
                            <ChevronsUpDown className="h-3 w-3 opacity-50" />
                          )}
                        </span>
                      )}
                    </span>
                  </th>
                );
              })}
            </tr>
          ))}
        </thead>
        <tbody>
          {loading ? (
            Array.from({ length: loadingRows }).map((_, i) => (
              <tr key={i} className="border-b border-border/50">
                {visibleColumns.map((_, j) => (
                  <td key={j} className="py-2 px-3">
                    <Skeleton className="h-4 w-full" />
                  </td>
                ))}
              </tr>
            ))
          ) : table.getRowModel().rows.length === 0 ? (
            <tr>
              <td colSpan={Math.max(1, visibleColumns.length)}>
                <EmptyState
                  title={emptyTitle}
                  description={emptyDescription}
                  className="py-12"
                />
              </td>
            </tr>
          ) : (
            table.getRowModel().rows.map((row) => {
              const isSelected = getRowIsSelected?.(row) ?? false;

              return (
                <tr
                  key={row.id}
                  data-state={isSelected ? 'selected' : undefined}
                  onClick={onRowClick ? () => onRowClick(row) : undefined}
                  className={cn(
                    'border-b border-border/50 transition-colors',
                    onRowClick && 'cursor-pointer hover:bg-bg-elevated',
                    isSelected && 'bg-accent/10 ring-1 ring-inset ring-accent/35 hover:bg-accent/15'
                  )}
                >
                  {row.getVisibleCells().map((cell) => {
                    const meta = cell.column.columnDef.meta as ColumnMeta | undefined;
                    return (
                    <td key={cell.id} className={cn('py-2 px-3 text-fg-secondary', isSelected && 'text-fg', meta?.cellClassName)}>
                      {flexRender(cell.column.columnDef.cell, cell.getContext())}
                    </td>
                    );
                  })}
                </tr>
              );
            })
          )}
        </tbody>
      </table>

      {columnVisibilityMenu && menuPosition && (
        <div
          className="fixed z-50 min-w-[180px] rounded-lg border border-border bg-bg-surface p-2 shadow-lg"
          style={{ left: menuPosition.x, top: menuPosition.y }}
          onClick={(event) => event.stopPropagation()}
        >
          <p className="px-2 pb-1 text-xs font-medium uppercase tracking-wide text-fg-tertiary">Columns</p>
          {menuColumns.map(({ column, label }) => {
            return (
              <label
                key={column.id}
                className="flex cursor-pointer items-center gap-2 rounded px-2 py-1 text-sm text-fg-secondary hover:bg-bg-elevated"
              >
                <input
                  type="checkbox"
                  checked={column.getIsVisible()}
                  onChange={column.getToggleVisibilityHandler()}
                  className="h-3.5 w-3.5 accent-accent"
                />
                <span>{label}</span>
              </label>
            );
          })}
        </div>
      )}
    </div>
  );
}
