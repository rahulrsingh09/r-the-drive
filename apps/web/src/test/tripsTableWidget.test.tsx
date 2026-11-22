import React from 'react';
import { fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const navigate = vi.fn();
const toggleTripSelection = vi.fn();
const mockTrips = vi.fn();
const selection = vi.hoisted(() => ({ selectedIds: [] as string[], tripRegistry: {} }));
const noTagIds = vi.hoisted(() => [] as string[]);

vi.mock('@tanstack/react-router', () => ({
  useNavigate: () => navigate,
}));

vi.mock('@tanstack/react-query', () => ({
  useQuery: vi.fn(() => ({ data: [] })),
}));

vi.mock('@panda/ui/primitives', async () => import('./mockPrimitives'));

vi.mock('@panda/hooks', () => ({
  api: { listPlaces: vi.fn(() => []) },
  useAuth: (selector?: (state: { accessToken: string | null }) => unknown) => {
    const state = { accessToken: null };
    return selector ? selector(state) : state;
  },
  useTrips: (...args: unknown[]) => mockTrips(...args),
  useTripMapRoutes: vi.fn(() => ({ isLoading: true, isError: false, data: undefined })),
  useBasemapConfig: () => ({ data: undefined, isError: false }),
  useUserPreferences: () => ({ data: { map_style: 'follow-theme' } }),
  useUpdateTripTagAssignments: () => ({ mutateAsync: vi.fn(), isPending: false, isError: false }),
}));

vi.mock('@panda/ui/charts', () => ({
  TripMapChart: () => <div data-testid="trip-map-chart" />,
}));

vi.mock('../../../../packages/dashboards/src/widgets/table/TripTagPicker', () => ({
  deriveCommonTagIds: () => noTagIds,
  TripTagBadges: () => null,
  TripTagPicker: () => null,
}));

vi.mock('../../../../packages/dashboards/src/widgets/table/tripSelectionStore', () => ({
  useTripSelection: () => selection,
  toggleTripSelection: (id: string) => toggleTripSelection(id),
  clearTripSelection: vi.fn(),
  resetTripSelection: vi.fn(),
  registerTripsInStore: vi.fn(),
}));

vi.mock('../../../../packages/dashboards/src/widgets/table/tripTableStateStore', () => ({
  useTripTableState: () => ({ page: 1, pageSize: 15, search: '' }),
  resetTripTableState: vi.fn(),
  setTripTablePage: vi.fn(),
  setTripTablePageSize: vi.fn(),
  setTripTableSearch: vi.fn(),
}));

import { TripsTableWidget } from '../../../../packages/dashboards/src/widgets/table/TripsTableWidget';

const trips = [
  {
    id: 'trip-default',
    started_at: '2026-05-01T12:00:00Z',
    ended_at: '2026-05-01T12:30:00Z',
    distance_mi: 12.4,
    duration_min: 30,
    energy_used_kwh: 4,
    efficiency_wh_mi: 320,
    soc_start: 75,
    soc_end: 68,
    drive_mode: 'all_purpose',
    start_place: 'Home',
    end_place: 'Office',
  },
  {
    id: 'trip-sport',
    started_at: '2026-05-02T12:00:00Z',
    ended_at: '2026-05-02T12:30:00Z',
    distance_mi: 20,
    duration_min: 30,
    energy_used_kwh: 7,
    efficiency_wh_mi: 350,
    soc_start: 68,
    soc_end: 55,
    drive_mode: 'sport',
  },
];

const ctx = { vehicleId: 'vehicle-1', from: '2026-05-01T00:00:00Z', to: '2026-05-31T23:59:59Z' } as never;

function setViewport(width: number) {
  Object.defineProperty(window, 'innerWidth', { configurable: true, value: width });
  window.matchMedia = vi.fn((query: string) => ({
    matches: width < 640 && query.includes('max-width: 639px'),
    media: query,
    onchange: null,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    addListener: vi.fn(),
    removeListener: vi.fn(),
    dispatchEvent: vi.fn(),
  })) as unknown as typeof window.matchMedia;
}

describe('TripsTableWidget trip list', () => {
  const originalMatchMedia = window.matchMedia;
  const originalWidth = window.innerWidth;

  beforeEach(() => {
    navigate.mockClear();
    toggleTripSelection.mockClear();
    mockTrips.mockReturnValue({ data: { items: trips, total: 2, per_page: 15 }, isLoading: false });
  });

  afterEach(() => {
    window.matchMedia = originalMatchMedia;
    Object.defineProperty(window, 'innerWidth', { configurable: true, value: originalWidth });
  });

  it('opens the trip from the whole card on a phone and has no Rows select', () => {
    setViewport(390);
    const { container } = render(<TripsTableWidget instance={{} as never} ctx={ctx} />);

    expect(screen.queryByLabelText('Trips per page')).not.toBeInTheDocument();
    expect(screen.queryByText('Rows')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Next' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Open trip details' })).not.toBeInTheDocument();
    expect(container.querySelector('.font-mono')).toBeNull();

    const cards = container.querySelectorAll('[data-trip-card]');
    expect(cards).toHaveLength(2);
    const first = within(cards[0] as HTMLElement);
    expect(first.getByText('75% → 68%')).toBeInTheDocument();
    expect(first.queryByText('All-Purpose')).not.toBeInTheDocument();
    expect(within(cards[1] as HTMLElement).getByText('Sport')).toBeInTheDocument();

    fireEvent.click(first.getByRole('button', { name: /^Open trip from / }));
    expect(navigate).toHaveBeenCalledWith({ to: '/trips/$tripId', params: { tripId: 'trip-default' } });

    fireEvent.click(first.getByRole('checkbox', { name: /^Select trip from / }));
    expect(toggleTripSelection).toHaveBeenCalledWith('trip-default');
    expect(navigate).toHaveBeenCalledTimes(1);
  });

  it('opens the trip from a desktop row and keeps selection on the checkbox', () => {
    setViewport(1280);
    render(<TripsTableWidget instance={{} as never} ctx={ctx} />);

    expect(screen.getByLabelText('Trips per page')).toBeInTheDocument();
    expect(screen.getByRole('columnheader', { name: 'Battery' })).toBeInTheDocument();
    expect(screen.getByRole('columnheader', { name: 'Mode' })).toBeInTheDocument();

    fireEvent.click(screen.getByText('Office'));
    expect(navigate).toHaveBeenCalledWith({ to: '/trips/$tripId', params: { tripId: 'trip-default' } });
    expect(toggleTripSelection).not.toHaveBeenCalled();

    fireEvent.click(screen.getAllByRole('checkbox', { name: /^Select trip from / })[1]!);
    expect(toggleTripSelection).toHaveBeenCalledWith('trip-sport');
    expect(navigate).toHaveBeenCalledTimes(1);
  });

  it('gives the search, pagination and trip checkboxes a 44 px target', () => {
    setViewport(390);
    const { container } = render(<TripsTableWidget instance={{} as never} ctx={ctx} />);

    expect(screen.getByRole('searchbox', { name: 'Search trips' })).toHaveClass('h-11');
    for (const name of ['Prev', 'Next']) {
      expect(screen.getByRole('button', { name })).toHaveClass('min-h-11', 'min-w-11');
    }
    const checkbox = within(container.querySelector('[data-trip-card]') as HTMLElement)
      .getByRole('checkbox', { name: /^Select trip from / });
    expect(checkbox).toHaveClass('absolute', 'inset-0', 'h-full', 'w-full');
    expect(checkbox.closest('label')).toHaveClass('h-11', 'w-11');
  });

  it('hides the Mode column when every visible trip uses the default mode', () => {
    setViewport(1280);
    mockTrips.mockReturnValue({ data: { items: [trips[0]], total: 1, per_page: 15 }, isLoading: false });
    render(<TripsTableWidget instance={{} as never} ctx={ctx} />);

    expect(screen.queryByRole('columnheader', { name: 'Mode' })).not.toBeInTheDocument();
    expect(screen.queryByText('All-Purpose')).not.toBeInTheDocument();
  });

  it.each([
    ['desktop', 1280],
    ['phone', 390],
  ])('shows Could not load trips · Retry on %s when the read fails, not No trips found', (_label, width) => {
    setViewport(width);
    const refetch = vi.fn();
    mockTrips.mockReturnValue({ data: undefined, isLoading: false, isError: true, refetch });
    render(<TripsTableWidget instance={{} as never} ctx={ctx} />);

    expect(screen.getByRole('alert')).toHaveTextContent('Could not load trips · Retry');
    expect(screen.queryByText('No trips found')).not.toBeInTheDocument();
    expect(screen.queryByRole('table')).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    expect(refetch).toHaveBeenCalledTimes(1);
  });
});
