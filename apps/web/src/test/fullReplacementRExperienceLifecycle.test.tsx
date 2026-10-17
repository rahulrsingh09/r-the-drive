import React from 'react';
import { act, cleanup, fireEvent, render, renderHook, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
const fixture = vi.hoisted(() => ({
  ready: true,
  snapshot: { isPlaceholderData: false, data: null as unknown },
  live: { status: null as unknown, connected: true, connectionState: 'connected' },
  subscribe: vi.fn(),
  logout: vi.fn(async () => {}),
  navigate: vi.fn(async () => {}),
  theme: { isPending: false, onModeChange: vi.fn() },
  toast: vi.fn(),
}));
vi.mock('@panda/hooks', () => ({
  useAuth: (select: (state: { accessToken: string; logout: typeof fixture.logout }) => unknown) => select({ accessToken: 'synthetic-token', logout: fixture.logout }),
  useResolvedVehicleSelection: () => ({ effectiveVehicleId: 'vehicle', vehicleSelectionReady: fixture.ready }),
  useCurrentVehicleStatus: () => fixture.snapshot,
  useVehicleStatus: (...args: unknown[]) => { fixture.subscribe(...args); return fixture.live; },
}));
vi.mock('@panda/ui/hooks', () => ({
  useDocumentTheme: () => false,
  DataPaletteProvider: ({ children }: React.PropsWithChildren) => <>{children}</>,
}));
vi.mock('@panda/dashboards', () => ({ WidgetPresentationProvider: ({ children }: React.PropsWithChildren) => <>{children}</> }));
vi.mock('../features/r-experience/RDashboardVehicle', () => ({ RDashboardVehicle: () => null }));
vi.mock('../features/r-experience/dataPalette', () => ({ readRDataPalette: () => ({ palette: 'rad' }) }));
vi.mock('../hooks/useThemePreferenceController', () => ({ useThemePreferenceController: () => fixture.theme }));
vi.mock('../components/feedback/toast', () => ({ emitToast: (value: unknown) => fixture.toast(value) }));
vi.mock('@tanstack/react-router', () => ({
  useLocation: () => ({ pathname: '/' }),
  useNavigate: () => fixture.navigate,
  Link: ({ to, search: _search, children, ...props }: React.PropsWithChildren<{ to: string; search?: unknown }>) => <a href={to} {...props}>{children}</a>,
}));
import { RExperienceRoot } from '../features/r-experience/RExperienceRoot';
import { RAppLayout } from '../features/r-experience/RAppLayout';
import { useRConnection } from '../features/r-experience/useRConnection';

beforeEach(() => {
  fixture.ready = true;
  fixture.snapshot.data = null;
  fixture.snapshot.isPlaceholderData = false;
  fixture.live.status = null;
  fixture.live.connected = true;
  fixture.live.connectionState = 'connected';
  localStorage.clear();
});
afterEach(() => { cleanup(); vi.useRealTimers(); vi.restoreAllMocks(); vi.clearAllMocks(); });

it('owns document theme classes and input mode only for the mounted experience', () => {
  const view = render(<RExperienceRoot><input aria-label="Editable" /><p>Body</p></RExperienceRoot>);
  expect(document.body).toHaveClass('r-experience');
  expect(document.documentElement).toHaveClass('r-interface');
  fireEvent.pointerDown(document.body);
  expect(document.body.dataset.inputMode).toBe('pointer');
  fireEvent.keyDown(screen.getByLabelText('Editable'), { key: 'a' });
  expect(document.body.dataset.inputMode).toBe('pointer');
  fireEvent.keyDown(document.body, { key: 'a', ctrlKey: true });
  expect(document.body.dataset.inputMode).toBe('pointer');
  fireEvent.keyDown(screen.getByLabelText('Editable'), { key: 'Tab' });
  expect(document.body.dataset.inputMode).toBe('keyboard');
  view.unmount();
  expect(document.body).not.toHaveClass('r-experience');
  expect(document.documentElement).not.toHaveClass('r-interface');
  expect(document.body.dataset.inputMode).toBeUndefined();
  fireEvent.pointerDown(document.body);
  expect(document.body.dataset.inputMode).toBeUndefined();
});

it('does not subscribe before selection is ready or show another vehicle/placeholder snapshot', () => {
  fixture.ready = false;
  fixture.snapshot.data = { vehicle_id: 'vehicle', battery_level: 5, auth_state: 'needs_reauth' };
  const view = renderHook(useRConnection);
  expect(fixture.subscribe).toHaveBeenLastCalledWith(null, null);
  expect(view.result.current.status).toBeNull();
  expect(view.result.current.feed).toBeNull();
  fixture.ready = true;
  fixture.snapshot.isPlaceholderData = true;
  fixture.live.status = { vehicle_id: 'vehicle', battery_level: 20, auth_state: 'needs_reauth' };
  view.rerender();
  expect(view.result.current.status).toBe(fixture.live.status);
  expect(view.result.current.feed).toBeNull();
  fixture.snapshot.isPlaceholderData = false;
  view.rerender();
  expect(view.result.current.status).toBe(fixture.snapshot.data);
  expect(view.result.current.feed?.title).toBe('Rivian feed disconnected');
  fixture.snapshot.data = { vehicle_id: 'other', battery_level: 50 };
  fixture.live.status = { vehicle_id: 'other', battery_level: 60 };
  view.rerender();
  expect(view.result.current.status).toBeNull();
});

it('throttles feed health notices per vehicle and issue, gates disconnected transports and clears timers', () => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date('2026-01-02T12:00:00Z'));
  const schedule = vi.spyOn(window, 'setInterval');
  const clear = vi.spyOn(window, 'clearInterval');
  fixture.snapshot.data = { vehicle_id: 'vehicle', auth_state: 'needs_reauth' };
  const view = renderHook(useRConnection);
  expect(fixture.toast).toHaveBeenCalledOnce();
  act(() => vi.advanceTimersByTime(14 * 60_000));
  expect(fixture.toast).toHaveBeenCalledOnce();
  act(() => vi.advanceTimersByTime(60_000));
  expect(fixture.toast).toHaveBeenCalledTimes(2);
  fixture.live.connected = false;
  view.rerender();
  expect(view.result.current.onlineState).toBe('connecting');
  expect(clear).toHaveBeenCalledWith(schedule.mock.results[0]!.value);
  view.unmount();
});

it('updates compact navigation at the existing breakpoint and keeps logout ordering', async () => {
  let matches = false;
  const listeners = new Set<() => void>();
  vi.spyOn(window, 'matchMedia').mockImplementation(query => ({
    get matches() { return matches; }, media: query,
    addEventListener: (_name: string, listener: () => void) => listeners.add(listener),
    removeEventListener: (_name: string, listener: () => void) => listeners.delete(listener),
  }) as unknown as MediaQueryList);
  const view = render(<RAppLayout activeKey="efficiency">Content</RAppLayout>);
  expect(screen.getByRole('link', { name: 'Efficiency' })).toHaveAttribute('aria-current', 'page');
  act(() => { matches = true; for (const listener of listeners) listener(); });
  expect(screen.queryByRole('link', { name: 'Efficiency' })).toBeNull();
  expect(screen.getByRole('link', { name: 'Explore' })).toHaveAttribute('aria-current', 'page');
  fireEvent.click(screen.getByLabelText('Switch to dark mode'));
  expect(fixture.theme.onModeChange).toHaveBeenCalledWith('dark');
  await act(async () => fireEvent.click(screen.getByLabelText('Sign out')));
  expect(fixture.logout.mock.invocationCallOrder[0]).toBeLessThan(fixture.navigate.mock.invocationCallOrder[0]!);
  expect(fixture.navigate).toHaveBeenCalledWith({ to: '/login' });
  view.unmount();
  expect(listeners.size).toBe(0);
});

it('does not leave a reminder scheduled when storage rejects the initial notification', () => {
  fixture.snapshot.data = { vehicle_id: 'vehicle', auth_state: 'needs_reauth' };
  const schedule = vi.spyOn(window, 'setInterval');
  vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => { throw new Error('Synthetic storage failure'); });
  expect(() => renderHook(useRConnection)).toThrow('Synthetic storage failure');
  expect(schedule).not.toHaveBeenCalled();
});
