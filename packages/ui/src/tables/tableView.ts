import type { Row, Table } from '@tanstack/react-table';
import { cn } from '../lib/utils';

export interface ColumnMeta {
  headerClassName?: string;
  cellClassName?: string;
  headerContentClassName?: string;
  columnLabel?: string;
}

export function tableView<T>(table: Table<T>, selected?: (row: Row<T>) => boolean, activate?: (row: Row<T>) => void, loading = false) {
  const columns = table.getAllLeafColumns();
  const menu = [];
  const sorting = [{ value: '', label: 'Original order' }];
  for (const column of columns) {
    const { header, meta } = column.columnDef;
    const metadata = meta as ColumnMeta | undefined;
    const label = typeof header === 'string' ? header : metadata?.columnLabel;
    if (typeof label !== 'string') continue;
    menu.push({ id: column.id, label, checked: column.getIsVisible(), onChange: column.getToggleVisibilityHandler() });
    if (column.getCanSort()) sorting.push({ value: column.id, label: metadata?.columnLabel ?? String(header) });
  }
  const headers = table.getHeaderGroups().map(group => ({
    id: group.id,
    cells: group.headers.map(header => {
      const column = header.column;
      const meta = column.columnDef.meta as ColumnMeta | undefined;
      const sortable = column.getCanSort();
      return {
        id: header.id, definition: column.columnDef.header, context: header.getContext(),
        sort: sortable ? column.getIsSorted() || 'none' as const : null,
        onClick: sortable ? column.getToggleSortingHandler() : undefined,
        className: cn('py-2 px-3 text-left text-xs font-medium text-fg-tertiary uppercase tracking-wider whitespace-nowrap', sortable && 'cursor-pointer select-none hover:text-fg-secondary', meta?.headerClassName),
        contentClassName: cn('flex items-center gap-1', meta?.headerContentClassName),
      };
    }),
  }));
  const rows = (loading ? [] : table.getRowModel().rows).map(row => {
    const active = selected?.(row) ?? false;
    return {
      row,
      state: active ? 'selected' : undefined,
      onClick: activate && (() => activate(row)),
      className: cn('border-b border-border/50 transition-colors', activate && 'cursor-pointer hover:bg-bg-elevated', active && 'bg-accent/10 ring-1 ring-inset ring-accent/35 hover:bg-accent/15'),
      cells: row.getVisibleCells().map(cell => ({
        id: cell.id, definition: cell.column.columnDef.cell, context: cell.getContext(),
        className: cn('py-2 px-3 text-fg-secondary', active && 'text-fg', (cell.column.columnDef.meta as ColumnMeta | undefined)?.cellClassName),
      })),
    };
  });
  return { menu, sorting, headers, rows, width: table.getVisibleLeafColumns().length };
}
