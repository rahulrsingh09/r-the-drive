import React from 'react';
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import { DataTable, createTripColumns, isDefaultTripDriveMode, type TripRow } from '@panda/ui/tables';

vi.mock('@panda/ui/primitives', async () => {
  const m = await import('./mockPrimitives');
  return m;
});

describe('trip columns', () => {
  it('keeps the compact row but gives the checkbox, date link and tag button a 44 px target', () => {
    const trip: TripRow = {
      id: 'trip-1',
      started_at: '2024-01-01T12:00:00Z',
      ended_at: '2024-01-01T13:00:00Z',
      distance_mi: 18.3,
      duration_min: 60,
      energy_used_kwh: 6.5,
      efficiency_wh_mi: 355,
      soc_start: 75,
      soc_end: 68,
      tags: [
        { id: 'tag-1', vehicle_id: 'vehicle-1', name: 'Home', color_token: 'accent', created_by: 'user-1', created_at: '', updated_at: '' },
        { id: 'tag-2', vehicle_id: 'vehicle-1', name: 'School', color_token: 'success', created_by: 'user-1', created_at: '', updated_at: '' },
      ],
    };
    const onToggleSelect = vi.fn();
    let selected = false;
    const { rerender } = render(
      <DataTable
        data={[trip]}
        columns={createTripColumns([], { onOpenTrip: vi.fn(), onToggleSelect, isSelected: () => selected, includeTags: true })}
      />,
    );

    const checkbox = screen.getByRole('checkbox', { name: /^Select trip from / });
    // The input itself is the 44 × 44 target; negative margins keep the 28 px footprint.
    expect(checkbox).toHaveClass('absolute', 'inset-0', 'h-full', 'w-full', 'appearance-none');
    expect(checkbox.closest('label')).toHaveClass('h-11', 'w-11', '-m-2');
    expect(checkbox).not.toBeChecked();
    fireEvent.click(checkbox);
    expect(onToggleSelect).toHaveBeenCalledWith('trip-1');
    selected = true;
    rerender(
      <DataTable
        data={[trip]}
        columns={createTripColumns([], { onOpenTrip: vi.fn(), onToggleSelect, isSelected: () => selected, includeTags: true })}
      />,
    );
    expect(screen.getByRole('checkbox', { name: /^Select trip from / })).toBeChecked();

    // Visually small controls take their target from their own ::before.
    expect(screen.getByRole('button', { name: /^Open trip from / })).toHaveClass('relative', 'before:absolute', 'before:h-11', 'before:inset-x-0');
    expect(screen.getByRole('button', { name: 'Show 2 tags for trip' })).toHaveClass('relative', 'before:absolute', 'before:h-11', 'before:w-11');
  });

  it('opens the whole trip from the row or the date, with no separate info button', () => {
    const trip: TripRow = {
      id: 'trip-1',
      started_at: '2024-01-01T12:00:00Z',
      ended_at: '2024-01-01T13:00:00Z',
      distance_mi: 18.3,
      duration_min: 60,
      energy_used_kwh: 6.5,
      efficiency_wh_mi: 355,
      soc_start: 75,
      soc_end: 68,
      drive_mode: 'all_purpose',
    };
    const onOpenTrip = vi.fn();
    const onToggleSelect = vi.fn();

    const { container } = render(
      <DataTable
        data={[trip]}
        columns={createTripColumns([], { onOpenTrip, onToggleSelect, isSelected: () => false })}
        onRowClick={(row) => onOpenTrip(row.original.id)}
      />,
    );

    expect(screen.queryByRole('button', { name: 'Open trip details' })).not.toBeInTheDocument();
    expect(container.querySelector('.font-mono')).toBeNull();
    expect(screen.getByRole('columnheader', { name: 'Battery' })).toBeInTheDocument();
    expect(screen.queryByRole('columnheader', { name: /SoC|SOC/ })).not.toBeInTheDocument();
    expect(screen.getByRole('columnheader', { name: 'Efficiency' })).toBeInTheDocument();
    expect(screen.getByText('75% → 68%')).toHaveClass('tabular-nums');
    expect(screen.queryByText('All-Purpose')).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: /^Open trip from / }));
    expect(onOpenTrip).toHaveBeenCalledTimes(1);
    expect(onOpenTrip).toHaveBeenLastCalledWith('trip-1');

    fireEvent.click(screen.getAllByRole('cell')[3]!);
    expect(onOpenTrip).toHaveBeenCalledTimes(2);

    fireEvent.click(screen.getByRole('checkbox', { name: /^Select trip from / }));
    expect(onToggleSelect).toHaveBeenCalledWith('trip-1');
    expect(onOpenTrip).toHaveBeenCalledTimes(2);
  });

  it('shows the drive mode chip only for a mode that is not the default', () => {
    const base: TripRow = {
      id: 'trip-mode',
      started_at: '2024-01-01T12:00:00Z',
      ended_at: '2024-01-01T13:00:00Z',
      distance_mi: 18.3,
      duration_min: 60,
      energy_used_kwh: 6.5,
      efficiency_wh_mi: 355,
      soc_start: 80,
      soc_end: 68,
    };

    expect(isDefaultTripDriveMode('All-Purpose')).toBe(true);
    expect(isDefaultTripDriveMode('everyday')).toBe(true);
    expect(isDefaultTripDriveMode(null)).toBe(true);
    expect(isDefaultTripDriveMode('sport')).toBe(false);

    render(<DataTable data={[{ ...base, drive_mode: 'sport' }, { ...base, id: 'trip-default', drive_mode: 'All-Purpose' }]} columns={createTripColumns()} />);
    expect(screen.getByText('Sport')).toBeInTheDocument();
    expect(screen.queryByText('All-Purpose')).not.toBeInTheDocument();

    expect(createTripColumns([], { includeMode: false }).some((column) => column.id === 'drive_mode' || (column as { accessorKey?: string }).accessorKey === 'drive_mode')).toBe(false);
  });

  it('uses a compact tag action only when the visible page has tags', () => {
    const withoutTags = createTripColumns([], { includeTags: false });
    expect(withoutTags.some((column) => column.id === 'tags')).toBe(false);

    const trip: TripRow = {
      id: 'trip-with-tags',
      started_at: '2024-01-01T12:00:00Z',
      ended_at: '2024-01-01T13:00:00Z',
      distance_mi: 18.3,
      duration_min: 60,
      energy_used_kwh: 6.5,
      efficiency_wh_mi: 355,
      soc_start: 80,
      soc_end: 68,
      tags: [
        { id: 'tag-1', vehicle_id: 'vehicle-1', name: 'Home', color_token: 'accent', created_by: 'user-1', created_at: '', updated_at: '' },
        { id: 'tag-2', vehicle_id: 'vehicle-1', name: 'School', color_token: 'success', created_by: 'user-1', created_at: '', updated_at: '' },
      ],
    };

    const { container } = render(
      <DataTable data={[trip]} columns={createTripColumns([], { includeTags: true })} />,
    );

    expect(screen.getByRole('columnheader', { name: 'Tags' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Show 2 tags for trip' })).toHaveClass('h-7', 'w-7');
    expect(container.querySelector('th:nth-last-child(1)')).not.toHaveClass('w-[11rem]');
  });

  it('opens the tag tooltip from keyboard focus with accessible canonical chips', () => {
    const trip: TripRow = {
      id: 'trip-tooltip',
      started_at: '2024-01-01T12:00:00Z',
      ended_at: '2024-01-01T13:00:00Z',
      distance_mi: 18.3,
      duration_min: 60,
      energy_used_kwh: 6.5,
      efficiency_wh_mi: 355,
      soc_start: 80,
      soc_end: 68,
      tags: [{ id: 'tag-1', vehicle_id: 'vehicle-1', name: 'school', color_token: 'accent', created_by: 'user-1', created_at: '', updated_at: '' }],
    };

    render(<DataTable data={[trip]} columns={createTripColumns([], { includeTags: true })} />);
    const trigger = screen.getByRole('button', { name: 'Show 1 tag for trip' });
    fireEvent.focus(trigger);

    const tooltip = screen.getByRole('tooltip');
    expect(tooltip).toHaveTextContent('School');
    expect(tooltip.querySelector('.lucide-tag')).toBeInTheDocument();
    expect(trigger).toHaveAttribute('aria-describedby');
    expect(tooltip).toHaveClass('max-w-[calc(100vw-1rem)]', 'max-h-[calc(100vh-1rem)]');

    fireEvent.keyDown(document, { key: 'Escape' });
    expect(screen.queryByRole('tooltip')).not.toBeInTheDocument();
  });

  it('shows explicit unavailable copy instead of blank or zero coordinates', () => {
    const trip: TripRow = {
      id: 'trip-2',
      started_at: '2024-01-01T12:00:00Z',
      ended_at: '2024-01-01T13:00:00Z',
      distance_mi: 18.3,
      duration_min: 60,
      energy_used_kwh: 6.5,
      efficiency_wh_mi: 355,
      soc_start: 80,
      soc_end: 68,
      drive_mode: 'All-Purpose',
      start_lat: 0,
      start_lng: 0,
      end_lat: 0,
      end_lng: 0,
    };

    render(
      <DataTable
        data={[trip]}
        columns={createTripColumns()}
      />,
    );

    expect(screen.queryByText('0.00000, 0.00000')).not.toBeInTheDocument();
    expect(screen.getAllByText('Location unavailable')).toHaveLength(2);
  });

  it('prefers a place label over address and coordinates', () => {
    const trip: TripRow = {
      id: 'trip-3',
      started_at: '2024-01-01T12:00:00Z',
      ended_at: '2024-01-01T13:00:00Z',
      distance_mi: 18.3,
      duration_min: 60,
      energy_used_kwh: 6.5,
      efficiency_wh_mi: 355,
      soc_start: 80,
      soc_end: 68,
      start_place: 'Home - Test',
      start_address: 'North Main Street, Houston, TX 77009',
      start_lat: 29.81831,
      start_lng: -95.38817,
      end_place: 'Aurora Street, Houston',
      end_address: 'Aurora Street, Houston, TX 77058',
      end_lat: 29.84793,
      end_lng: -95.50235,
    };

    render(<DataTable data={[trip]} columns={createTripColumns()} />);

    expect(screen.getByText('Home - Test')).toBeInTheDocument();
    expect(screen.getByText('Aurora Street, Houston')).toBeInTheDocument();
    expect(screen.queryByText('29.81831, -95.38817')).not.toBeInTheDocument();
    expect(screen.queryByText('North Main Street, Houston, TX 77009')).not.toBeInTheDocument();
  });

  it('falls back to address text before raw coordinates', () => {
    const trip: TripRow = {
      id: 'trip-4',
      started_at: '2024-01-01T12:00:00Z',
      ended_at: '2024-01-01T13:00:00Z',
      distance_mi: 18.3,
      duration_min: 60,
      energy_used_kwh: 6.5,
      efficiency_wh_mi: 355,
      soc_start: 80,
      soc_end: 68,
      start_address: 'North Main Street, Houston, TX 77009',
      start_lat: 29.81831,
      start_lng: -95.38817,
      end_address: 'Aurora Street, Houston, TX 77058',
      end_lat: 29.84793,
      end_lng: -95.50235,
    };

    render(<DataTable data={[trip]} columns={createTripColumns()} />);

    expect(screen.getByText('North Main Street, Houston, TX 77009')).toBeInTheDocument();
    expect(screen.getByText('Aurora Street, Houston, TX 77058')).toBeInTheDocument();
    expect(screen.queryByText('29.81831, -95.38817')).not.toBeInTheDocument();
    expect(screen.queryByText('29.84793, -95.50235')).not.toBeInTheDocument();
  });

  it('falls back to coordinates only when no place or address exists', () => {
    const trip: TripRow = {
      id: 'trip-5',
      started_at: '2024-01-01T12:00:00Z',
      ended_at: '2024-01-01T13:00:00Z',
      distance_mi: 18.3,
      duration_min: 60,
      energy_used_kwh: 6.5,
      efficiency_wh_mi: 355,
      soc_start: 80,
      soc_end: 68,
      start_lat: 29.81831,
      start_lng: -95.38817,
      end_lat: 29.84793,
      end_lng: -95.50235,
    };

    render(<DataTable data={[trip]} columns={createTripColumns()} />);

    expect(screen.getByText('29.81831, -95.38817')).toBeInTheDocument();
    expect(screen.getByText('29.84793, -95.50235')).toBeInTheDocument();
  });
});
