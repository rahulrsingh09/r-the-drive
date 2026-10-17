import React from 'react';
import { act, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  getDoorOverlayUrls,
  getOpenDoorStates,
  findFirstSideImage,
  findFirstOverheadImage,
  findSideChargingImage,
  findBestChargingSideOverlay,
} from '../../../../packages/dashboards/src/widgets/custom/imageUtils';
import {
  computeTripStat,
  formatTripStat,
} from '../../../../packages/dashboards/src/widgets/table/tripStatMetrics';
import {
  clearTripSelection,
  registerTripsInStore,
  resetTripSelection,
  toggleTripSelection,
  useTripSelection,
} from '../../../../packages/dashboards/src/widgets/table/tripSelectionStore';
import {
  resetTripTableState,
  setTripTablePage,
  setTripTablePageSize,
  setTripTableSearch,
  useTripTableState,
} from '../../../../packages/dashboards/src/widgets/table/tripTableStateStore';
import { useMeasuredWidgetHeight } from '../../../../packages/dashboards/src/widgets/useMeasuredWidgetHeight';
import type { VehicleImages, VehicleStatus } from '@panda/types';
import type { TripRow } from '@panda/ui/tables';
const trip = (
  id: string,
  distance_mi: number,
  efficiency_wh_mi: number | null,
  duration_min = 10
) => ({ id, distance_mi, efficiency_wh_mi, duration_min }) as TripRow;
const image = (placement: string, design: string, url: string, metadata = {}) =>
  ({ placement, design, url, metadata }) as VehicleImages['all'][number];
afterEach(() => vi.restoreAllMocks());
describe('dashboard widget contracts', () => {
  it('retains selection order, silent trip registration, context resets and page resets', () => {
    resetTripSelection('one', { force: true });
    resetTripTableState('one', { force: true });
    const { result } = renderHook(() => ({
      selection: useTripSelection(),
      table: useTripTableState(),
    }));
    act(() => {
      registerTripsInStore([trip('a', 1, 100), trip('b', 2, null)]);
      toggleTripSelection('b');
      toggleTripSelection('a');
    });
    expect(result.current.selection.selectedIds).toEqual(['b', 'a']);
    expect(result.current.selection.tripRegistry.a!.distance_mi).toBe(1);
    act(() => {
      toggleTripSelection('b');
      setTripTablePage(5);
    });
    expect(result.current.selection.selectedIds).toEqual(['a']);
    expect(result.current.table.page).toBe(5);
    act(() => {
      resetTripSelection('one');
      resetTripTableState('one');
    });
    expect(result.current.selection.selectedIds).toEqual(['a']);
    expect(result.current.table.page).toBe(5);
    act(() => setTripTableSearch(' mountain '));
    expect(result.current.table).toEqual({ page: 1, pageSize: 15, search: ' mountain ' });
    act(() => {
      setTripTablePage(3);
      setTripTablePageSize(25);
      clearTripSelection();
    });
    expect(result.current.table).toEqual({ page: 1, pageSize: 25, search: ' mountain ' });
    expect(Object.keys(result.current.selection.tripRegistry)).toEqual(['a', 'b']);
    act(() => {
      resetTripSelection('two');
      resetTripTableState('two');
    });
    expect(result.current.selection).toEqual({ selectedIds: [], tripRegistry: {} });
    expect(result.current.table).toEqual({ page: 1, pageSize: 15, search: '' });
  });
  it('weights only trips with positive distance and a non-null efficiency', () => {
    const trips = [
      trip('a', 10, 200, 5),
      trip('b', 30, 400, 15),
      trip('c', 0, 1000, 20),
      trip('d', -4, 800, 0),
      trip('e', 4, null, 10),
    ];
    expect(computeTripStat('miles', trips)).toBe(40);
    expect(computeTripStat('duration', trips)).toBe(10);
    expect(computeTripStat('count', trips)).toBe(5);
    expect(computeTripStat('efficiency', trips)).toBe(350);
    expect(computeTripStat('efficiency', [trip('a', 0, 4)])).toBeNull();
    expect(computeTripStat('count', [])).toBeNull();
    expect(formatTripStat('miles', Infinity)).toBe('-');
  });
  it('orders closure overlays and prefers design before alternative door names', () => {
    expect(
      getOpenDoorStates({
        door_front_right_closed: false,
        closure_liftgate_closed: false,
        closure_tailgate_closed: false,
        side_bin_left_closed: false,
      } as VehicleStatus)
    ).toEqual(['front_right', 'rear_gate', 'side_bin_left']);
    expect(getOpenDoorStates(undefined)).toEqual([]);
    const images = [
      image('overhead tailgate open', 'light', 'tail'),
      image('overhead liftgate open', 'dark', 'lift'),
      image('overhead front right open', 'dark', 'right'),
      image('overhead side_bin_left_open', 'dark', 'bin'),
    ];
    expect(getDoorOverlayUrls(images, ['rear_gate', 'front_right', 'rear_gate'], 'dark')).toEqual([
      'lift',
      'right',
    ]);
    expect(findFirstOverheadImage(images, 'dark')).toBe('lift');
    expect(getDoorOverlayUrls(undefined, ['frunk'], 'light')).toEqual([]);
  });
  it('preserves placement precedence and first-match empty URL fallback', () => {
    const images = [
      image('side', 'light', 'base'),
      image('side charging', 'dark', 'charge'),
      image('side', 'dark', ''),
      image('side', 'dark', 'later'),
      image('side port open', 'light', 'port'),
      image('side charging light', 'dark', 'light'),
    ];
    expect(findFirstSideImage(images, 'dark')).toBe('base');
    expect(findSideChargingImage(images, 'dark')).toBe('charge');
    expect(findBestChargingSideOverlay(images, 'dark')).toBe('light');
  });
  it('measures positive height, floors pixels, clamps output and disconnects', () => {
    let resize = () => {};
    const disconnect = vi.fn();
    vi.stubGlobal(
      'ResizeObserver',
      class {
        constructor(cb: () => void) {
          resize = cb;
        }
        observe() {}
        disconnect = disconnect;
      }
    );
    let measured = 200.9;
    const node = document.createElement('div');
    node.getBoundingClientRect = () => ({ height: measured }) as DOMRect;
    const { result, unmount } = renderHook(() => {
      const measuredHeight = useMeasuredWidgetHeight(260, 140);
      measuredHeight.ref.current = node;
      return measuredHeight;
    });
    expect(result.current.height).toBe(200);
    act(() => {
      measured = 0;
      resize();
    });
    expect(result.current.height).toBe(200);
    act(() => {
      measured = 70;
      resize();
    });
    expect(result.current.height).toBe(140);
    unmount();
    expect(disconnect).toHaveBeenCalledOnce();
    vi.unstubAllGlobals();
  });
});
