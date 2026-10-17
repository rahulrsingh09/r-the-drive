import React from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { Button, Input, MetricTabs, StatCard, ChartPicker, Switch, Sidebar } from '@panda/ui/primitives';
import { DataTable, TableControls, createTripColumns, type TripRow } from '@panda/ui/tables';
import { whPerMileToMiPerKwh, whPerMileToKmPerKwh, whPerMileToWhPerKm } from '@panda/ui/lib/utils';
import { resolveTripLocation } from '@panda/ui/lib/tripPresentation';
import { contrastRatio } from '@panda/ui/lib/color';
import type { Place } from '@panda/types';

afterEach(() => { cleanup(); vi.restoreAllMocks(); });

describe('foundation recovery boundaries', () => {
  it('does not inspect row selection while loading, then preserves selection through sorting and visibility', () => {
    const selected = vi.fn((row: { original: { name: string } }) => row.original.name === 'Beta');
    const click = vi.fn();
    const props = { data: [{ name: 'Beta', value: 2 }, { name: 'Alpha', value: 1 }], columns: [{ accessorKey: 'name', header: 'Name' }, { accessorKey: 'value', header: 'Value' }], getRowIsSelected: selected, onRowClick: click, columnVisibilityMenu: true };
    const view = render(<DataTable {...props} loading loadingRows={2} />);
    expect(selected).not.toHaveBeenCalled();
    expect(screen.getAllByRole('row')).toHaveLength(3);
    view.rerender(<DataTable {...props} />);
    const beta = screen.getByText('Beta').closest('tr')!;
    expect(beta).toHaveAttribute('data-state', 'selected');
    expect(within(beta).getByText('2')).toHaveClass('text-fg');
    fireEvent.click(screen.getByRole('columnheader', { name: 'Name' }));
    expect(screen.getAllByRole('row').slice(1).map(row => row.textContent)).toEqual(['Alpha1', 'Beta2']);
    fireEvent.click(screen.getByText('Beta'));
    expect(click.mock.calls[0]![0].original.name).toBe('Beta');
    fireEvent.contextMenu(screen.getByRole('columnheader', { name: 'Name' }));
    fireEvent.click(screen.getByRole('checkbox', { name: 'Value' }));
    expect(screen.queryByRole('columnheader', { name: 'Value' })).toBeNull();
    expect(screen.getByText('Beta').closest('tr')).toHaveAttribute('data-state', 'selected');
  });

  it('keeps an empty desktop cell even with all columns hidden and caps mobile loading cards', () => {
    const props = { data: [] as { value: number }[], columns: [{ accessorKey: 'value', header: 'Value' }], defaultHiddenColumns: ['value'], renderMobileRow: () => <span>Row</span> };
    const view = render(<DataTable {...props} loading loadingRows={8} />);
    expect(screen.getByTestId('mobile-table-cards').querySelectorAll('.h-36')).toHaveLength(5);
    view.rerender(<DataTable {...props} />);
    expect(screen.getByRole('cell')).toHaveAttribute('colspan', '1');
    expect(screen.getAllByText('No data')).toHaveLength(2);
  });

  it('selects optional trip columns independently and isolates the detail action', () => {
    const detail = vi.fn(), row = vi.fn();
    const trip: TripRow = { id: 'synthetic-trip', started_at: '2026-10-01T12:00:00Z', ended_at: null, duration_min: 5, distance_mi: 1, energy_used_kwh: null, efficiency_wh_mi: null, soc_start: null, soc_end: null };
    for (const includeTags of [false, true]) {
      const columns = createTripColumns([], { includeTags, onInfoClick: detail });
      const view = render(<DataTable data={[trip]} columns={columns} onRowClick={row} />);
      expect(Boolean(screen.queryByRole('columnheader', { name: 'Tags' }))).toBe(includeTags);
      fireEvent.click(screen.getByRole('button', { name: 'Open trip details' }));
      expect(detail).toHaveBeenLastCalledWith('synthetic-trip');
      expect(row).not.toHaveBeenCalled();
      view.unmount();
    }
    render(<DataTable data={[trip]} columns={createTripColumns([], { includeTags: false })} />);
    expect(screen.queryByRole('button', { name: 'Open trip details' })).toBeNull();
  });

  it('reports controlled table search and numeric page sizes without altering page bounds', () => {
    const search = vi.fn(), rows = vi.fn();
    const view = render(<TableControls search="" onSearchChange={search} searchPlaceholder="Find" rowsPerPage={10} rowsPerPageOptions={[10, 25]} onRowsPerPageChange={rows} page={0} totalPages={0} totalItems={1} itemLabel="trip" />);
    expect(screen.getByText('Page 0 of 1 | 1 trip')).toBeInTheDocument();
    fireEvent.change(screen.getByRole('searchbox', { name: 'Search trip' }), { target: { value: 'station' } });
    expect(search).toHaveBeenCalledWith('station');
    fireEvent.click(screen.getByRole('button', { name: 'Rows per page' }));
    fireEvent.click(screen.getByRole('option', { name: '25' }));
    expect(rows).toHaveBeenCalledWith(25);
    view.unmount();
  });

  it('keeps loading button adornments and input error precedence', () => {
    const view = render(<><Button loading iconLeft="Left" iconRight="Right">Save</Button><Input label="Field name" error="Invalid" hint="Hint" iconLeft="L" iconRight="R" /></>);
    expect(screen.getByRole('button', { name: 'Save' })).toBeDisabled();
    expect(screen.queryByText('Left')).toBeNull();
    expect(screen.queryByText('Right')).toBeNull();
    expect(screen.getByLabelText('Field name')).toHaveAttribute('id', 'field-name');
    expect(screen.queryByText('Hint')).toBeNull();
    expect(screen.getByText('Invalid')).toHaveClass('text-status-danger');
    view.rerender(<><Button iconLeft="Left" iconRight="Right">Save</Button><Input label="Field name" hint="Hint" /></>);
    expect(screen.getByText('Left')).toBeInTheDocument();
    expect(screen.getByText('Right')).toBeInTheDocument();
    expect(screen.getByText('Hint')).toHaveClass('text-fg-tertiary');
  });

  it('switches tab presentation only beyond the threshold and preserves delta precedence', () => {
    const change = vi.fn();
    const tabs = [{ key: 'a', label: 'Alpha' }, { key: 'b', label: 'Beta' }];
    const view = render(<MetricTabs tabs={tabs} active="a" onChange={change} dropdownThreshold={2}>Content</MetricTabs>);
    fireEvent.click(screen.getByRole('button', { name: 'Beta' }));
    expect(change).toHaveBeenCalledWith('b');
    view.rerender(<MetricTabs tabs={tabs} active="a" onChange={change} dropdownThreshold={1}>Content</MetricTabs>);
    expect(screen.getByRole('button', { name: 'Select metric' })).toBeInTheDocument();
    view.rerender(<StatCard label="Distance" value={1} delta={0} detail="Detail" />);
    expect(screen.queryByText('Detail')).toBeNull();
    expect(screen.getByText('0')).toBeInTheDocument();
    view.rerender(<StatCard label="Distance" value={1} detail="Detail" />);
    expect(screen.getByText('Detail')).toBeInTheDocument();
  });

  it('does not select a chart when marking its default, in either picker presentation', () => {
    for (const variant of ['default', 'compact'] as const) {
      const change = vi.fn(), favorite = vi.fn();
      const view = render(<ChartPicker variant={variant} value="a" options={[{ value: 'a', label: 'Alpha' }, { value: 'b', label: 'Beta' }]} defaultValue="a" onChange={change} onSetDefault={favorite} searchValue="" onSearchChange={vi.fn()} />);
      fireEvent.click(screen.getByRole('button', { name: 'Chart' }));
      expect(screen.getByRole('button', { name: 'Alpha is the default chart' })).toBeDisabled();
      fireEvent.click(screen.getByRole('button', { name: 'Set Beta as default' }));
      expect(favorite).toHaveBeenCalledWith('b');
      expect(change).not.toHaveBeenCalled();
      expect(screen.getByRole('listbox')).toBeInTheDocument();
      view.unmount();
    }
  });

  it('honors canceled switches and controlled sidebar collapse', () => {
    const change = vi.fn();
    const view = render(<Switch checked onChange={change} onClick={event => event.preventDefault()} aria-label="Control" />);
    fireEvent.click(screen.getByRole('switch'));
    expect(change).not.toHaveBeenCalled();
    view.rerender(<Sidebar activeKey="dashboard" onNavigate={vi.fn()} collapsed onCollapsedChange={change} />);
    fireEvent.click(screen.getByRole('button', { name: 'Expand sidebar' }));
    expect(change).toHaveBeenCalledWith(false);
    expect(screen.getByRole('button', { name: 'Expand sidebar' })).toBeInTheDocument();
  });

  it('preserves finite and sign boundaries for efficiency conversion', () => {
    for (const value of [null, undefined, NaN, Infinity, -Infinity]) {
      expect(whPerMileToMiPerKwh(value)).toBeNull();
      expect(whPerMileToKmPerKwh(value)).toBeNull();
      expect(whPerMileToWhPerKm(value)).toBeNull();
    }
    for (const value of [0, -0, -1]) expect(whPerMileToMiPerKwh(value)).toBeNull();
    expect(Object.is(whPerMileToWhPerKm(-0), -0)).toBe(true);
    expect(whPerMileToMiPerKwh(400)).toBe(2.5);
    expect(whPerMileToKmPerKwh(400)).toBe(2.5 * 1.609344);
    expect(whPerMileToWhPerKm(-1)).toBe(-1 / 1.609344);
    expect(contrastRatio('rgb(10.31475,10.31475,10.31475)', '#ffffff')).toBeCloseTo(1.05 / (0.04045 / 12.92 + 0.05), 12);
  });

  it('preserves inclusive place radius and stable first-place ties', () => {
    const point = { start_lat: 10, start_lng: 20 };
    const first = { id: 'first', name: 'First', latitude: 10, longitude: 20, radius_m: 0 } as Place;
    const second = { ...first, id: 'second', name: 'Second' };
    expect(resolveTripLocation(point, 'start', [first, second]).label).toBe('First');
    expect(resolveTripLocation(point, 'start', [{ ...first, latitude: NaN }, second]).label).toBe('Second');
    expect(resolveTripLocation(point, 'start', [{ ...first, radius_m: -1 }]).source).toBe('coordinates');
  });
});
