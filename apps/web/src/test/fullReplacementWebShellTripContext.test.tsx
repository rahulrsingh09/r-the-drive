import { act, renderHook } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';

const state = vi.hoisted(() => ({
  navigate: vi.fn(),
  search: { tag_ids: 'one,two', tag_match: 'all' },
  selected: 'v' as string | null,
  vehicles: [] as Array<{ id: string; membership_role: string }>,
}));

vi.mock('@tanstack/react-router', () => ({
  useSearch: () => state.search,
  useNavigate: () => state.navigate,
}));
vi.mock('@panda/hooks', () => ({
  useResolvedVehicleSelection: () => ({ effectiveVehicleId: state.selected, vehicles: state.vehicles }),
}));

import { useTripPageContext } from '../components/dashboard/useTripPageContext';

beforeEach(() => {
  state.navigate.mockReset();
  state.selected = 'v';
  state.vehicles = [];
});

it.each(['owner', 'manager', 'viewer', 'user', ''])('checks the selected membership %j for both trip surfaces', (role) => {
  state.vehicles = [{ id: 'other', membership_role: 'owner' }, { id: 'v', membership_role: role }];
  for (const route of ['/trips', '/efficiency'] as const) {
    const { result, unmount } = renderHook(() => useTripPageContext(route));
    expect(result.current.canManageTripTags).toBe(role === 'owner' || role === 'manager');
    unmount();
  }
});

it('uses the first matching membership and removes management when selection is absent', () => {
  state.vehicles = [{ id: 'v', membership_role: 'viewer' }, { id: 'v', membership_role: 'owner' }];
  const { result, rerender } = renderHook(() => useTripPageContext('/trips'));
  expect(result.current.canManageTripTags).toBe(false);
  state.vehicles.reverse();
  rerender();
  expect(result.current.canManageTripTags).toBe(true);
  state.selected = null;
  rerender();
  expect(result.current.canManageTripTags).toBe(false);
});

it('keeps filter navigation bound to the active route and replaces search history', () => {
  const { result, rerender } = renderHook(({ route }: { route: '/trips' | '/efficiency' }) => useTripPageContext(route), { initialProps: { route: '/trips' } });
  act(() => result.current.tripTagFilter.setFilter?.({ tagIds: ['two'], tagMatch: 'any', untagged: false }));
  expect(state.navigate).toHaveBeenLastCalledWith({ to: '/trips', search: { tag_ids: 'two', tag_match: 'any' }, replace: true });
  rerender({ route: '/efficiency' });
  act(() => result.current.tripTagFilter.setFilter?.({ tagIds: [], tagMatch: 'all', untagged: true }));
  expect(state.navigate).toHaveBeenLastCalledWith({ to: '/efficiency', search: { untagged: '1' }, replace: true });
});
