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
import { tableView } from './tableView';
import { useDataTableState } from '../hooks/useDataTableState';
import { Skeleton } from '../primitives/Skeleton';
import { EmptyState } from '../primitives/EmptyState';
import { SelectPicker } from '../primitives/SelectPicker';

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

  const view = tableView(table, getRowIsSelected, onRowClick, loading);
  const sortIcons = {
    asc: <ChevronUp className="h-3 w-3" />,
    desc: <ChevronDown className="h-3 w-3" />,
    none: <ChevronsUpDown className="h-3 w-3 opacity-50" />,
  };
  const direction = sorting[0]?.desc === true;
  const mobileDirection = direction
    ? <ChevronDown aria-hidden="true" className="h-4 w-4" />
    : <ChevronUp aria-hidden="true" className="h-4 w-4" />;
  let mobileRows: React.ReactNode = null;
  let body: React.ReactNode;
  if (loading) {
    const placeholders = Array.from({ length: loadingRows }, (_, index) => index);
    mobileRows = placeholders.slice(0, 5).map(index => <Skeleton key={index} className="h-36" />);
    const cells = Array.from({ length: view.width }, (_, index) => <td key={index} className="py-2 px-3"><Skeleton className="h-4 w-full" /></td>);
    body = placeholders.map(index => <tr key={index} className="border-b border-border/50">{cells}</tr>);
  } else if (view.rows.length === 0) {
    mobileRows = <EmptyState title={emptyTitle} description={emptyDescription} />;
    body = <tr><td colSpan={Math.max(1, view.width)}><EmptyState title={emptyTitle} description={emptyDescription} className="py-12" /></td></tr>;
  } else {
    if (renderMobileRow) mobileRows = view.rows.map(({ row }) => <React.Fragment key={row.id}>{renderMobileRow(row)}</React.Fragment>);
    body = view.rows.map(item => (
      <tr key={item.row.id} data-state={item.state} onClick={item.onClick} className={item.className}>
        {item.cells.map(cell => <td key={cell.id} className={cell.className}>{flexRender(cell.definition, cell.context)}</td>)}
      </tr>
    ));
  }

  return (
    <div className={cn('relative w-full overflow-x-auto', className)}>
      {renderMobileRow && (
        <div className="grid gap-3 py-3 md:hidden" data-testid="mobile-table-cards">
          <div className="flex items-center gap-2">
            <SelectPicker aria-label="Sort rows by" className="min-w-0 flex-1" value={sorting[0]?.id ?? ''} onChange={chooseSort} options={view.sorting} />
            <button type="button" aria-label={direction ? 'Sort ascending' : 'Sort descending'} disabled={!sorting.length} onClick={reverseSort} className="flex h-11 w-11 shrink-0 items-center justify-center rounded-lg border border-border bg-bg-surface text-fg-secondary transition-colors hover:bg-bg-elevated disabled:opacity-40">
              {mobileDirection}
            </button>
          </div>
          {mobileRows}
        </div>
      )}
      <table className={cn('w-full text-sm', fixedLayout && 'table-fixed', renderMobileRow && 'hidden md:table')}>
        <thead onContextMenu={openColumnMenu}>
          {view.headers.map(group => <tr key={group.id} className="border-b border-border">
            {group.cells.map(header => <th key={header.id} className={header.className} onClick={header.onClick} onContextMenu={openColumnMenu}>
              <span className={header.contentClassName}>
                {flexRender(header.definition, header.context)}
                {header.sort && <span className="text-fg-tertiary">{sortIcons[header.sort]}</span>}
              </span>
            </th>)}
          </tr>)}
        </thead>
        <tbody>{body}</tbody>
      </table>
      {columnVisibilityMenu && menuPosition && (
        <div className="fixed z-50 min-w-[180px] rounded-lg border border-border bg-bg-surface p-2 shadow-lg" style={{ left: menuPosition.x, top: menuPosition.y }} onClick={event => event.stopPropagation()}>
          <p className="px-2 pb-1 text-xs font-medium uppercase tracking-wide text-fg-tertiary">Columns</p>
          {view.menu.map(column => <label key={column.id} className="flex cursor-pointer items-center gap-2 rounded px-2 py-1 text-sm text-fg-secondary hover:bg-bg-elevated">
            <input type="checkbox" checked={column.checked} onChange={column.onChange} className="h-3.5 w-3.5 accent-accent" />
            <span>{column.label}</span>
          </label>)}
        </div>
      )}
    </div>
  );
}
