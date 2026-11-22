import React from 'react';
import { act, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { RConnectionBanner } from '../features/r-experience/useRConnection';

const mocks = vi.hoisted(() => ({
  pathname: '/trips',
  banner: null as RConnectionBanner | null,
  setMode: vi.fn(),
}));

vi.mock('@tanstack/react-router', () => ({
  Link: ({ to, search, children, ...props }: { to: string; search?: Record<string, string>; children: React.ReactNode }) =>
    <a href={`${to}${search ? `?${new URLSearchParams(search)}` : ''}`} {...props}>{children}</a>,
  useLocation: () => ({ pathname: mocks.pathname }),
}));

vi.mock('../features/r-experience/useRConnection', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../features/r-experience/useRConnection')>()),
  useRConnection: () => ({ vehicleId: 'v1', status: null, onlineState: 'online', feed: null, renewal: null, banner: mocks.banner }),
}));

vi.mock('../hooks/useThemePreferenceController', () => ({
  useThemePreferenceController: () => ({ mode: 'dark', onModeChange: mocks.setMode, isPending: false }),
}));

import { RAppLayout, RPageActions, RPageHeader, useHidePageActions } from '../features/r-experience/RAppLayout';

beforeEach(() => {
  mocks.pathname = '/trips';
  mocks.banner = null;
  mocks.setMode.mockClear();
});
afterEach(() => { document.title = ''; });

describe('RAppLayout shell', () => {
  it('names the four tabs and marks the active one', () => {
    render(<RAppLayout activeKey="activity"><p>Page</p></RAppLayout>);
    const navigation = screen.getByRole('navigation', { name: 'Primary navigation' });
    const links = within(navigation).getAllByRole('link');
    expect(links.map((link) => link.getAttribute('aria-label'))).toEqual(['Now', 'Activity', 'Insights', 'Car']);
    expect(within(navigation).getByRole('link', { name: 'Activity' })).toHaveAttribute('aria-current', 'page');
    expect(within(navigation).getByRole('link', { name: 'Now' })).not.toHaveAttribute('aria-current');
    expect(document.title).toBe('Activity · R');
  });

  it('has Settings in the rail and a gear in the page header, and a theme switch', () => {
    render(<RAppLayout activeKey="activity"><RPageHeader title="Activity" sub="Trips, charges and parked time" /></RAppLayout>);
    const settings = screen.getAllByRole('link', { name: 'Settings' });
    expect(settings.map(link => link.className)).toEqual(['r-tab r-rail-settings', 'r-iconbtn r-gearbtn']);
    expect(screen.getAllByRole('button', { name: 'Switch to light mode' })).toHaveLength(1);
    act(() => screen.getByRole('button', { name: 'Switch to light mode' }).click());
    expect(mocks.setMode).toHaveBeenCalledWith('light');
    expect(screen.getByRole('heading', { level: 1, name: 'Activity' })).toBeInTheDocument();
  });

  it('shows the buttons at the top right when a page has no RPageHeader', () => {
    render(<RAppLayout activeKey="activity"><h1>Old page</h1></RAppLayout>);
    expect(screen.getAllByRole('button', { name: 'Switch to light mode' })).toHaveLength(1);
  });

  it('keeps the buttons in a page title row with RPageActions, also under a banner', () => {
    mocks.banner = { kind: 'rivian', since: null, reason: 'credentials' };
    render(<RAppLayout activeKey="insights"><header data-testid="title-row"><h1>Insights</h1><RPageActions /></header></RAppLayout>);
    const buttons = screen.getAllByRole('button', { name: 'Switch to light mode' });
    expect(buttons).toHaveLength(1);
    expect(within(screen.getByTestId('title-row')).getByRole('button', { name: 'Switch to light mode' })).toBe(buttons[0]);
    expect(within(screen.getByTestId('title-row')).getByRole('link', { name: 'Settings' })).toHaveClass('r-gearbtn');
  });

  it('shows no gear and theme row on a page that hides it', () => {
    function DetailPage() { useHidePageActions(); return <h2>Trip · 1 of 2</h2>; }
    render(<RAppLayout activeKey="activity"><DetailPage /></RAppLayout>);
    expect(screen.queryByRole('button', { name: 'Switch to light mode' })).toBeNull();
  });

  it('marks Settings active and hides the gear on the Settings page', () => {
    mocks.pathname = '/settings';
    render(<RAppLayout activeKey="settings"><RPageHeader title="Settings" /></RAppLayout>);
    expect(screen.getAllByRole('link', { name: 'Settings' })).toHaveLength(1);
    expect(screen.getByRole('link', { name: 'Settings' })).toHaveAttribute('aria-current', 'page');
    expect(within(screen.getByRole('navigation', { name: 'Primary navigation' })).queryByRole('link', { current: 'page' })).toBeNull();
  });

  it('shows the Rivian connection banner with Reconnect', () => {
    mocks.banner = { kind: 'rivian', since: null, reason: 'credentials' };
    render(<RAppLayout activeKey="now"><p>Page</p></RAppLayout>);
    const alert = screen.getByRole('alert');
    expect(alert).toHaveTextContent('Rivian connection lost.');
    expect(alert).toHaveTextContent('After you reconnect, R recovers the missed charges from Rivian.');
    expect(within(alert).getByRole('link', { name: 'Reconnect' })).toHaveAttribute('href', '/connect?mode=refresh&vehicle_id=v1');
  });

  it('shows the offline banner', () => {
    mocks.banner = { kind: 'offline', since: null };
    render(<RAppLayout activeKey="now"><p>Page</p></RAppLayout>);
    expect(screen.getByRole('status')).toHaveTextContent("You're offline. Showing the last data R loaded. It updates when you reconnect.");
  });
});

describe('RAppLayout page slots', () => {
  it('renders the footer slot after the page body', () => {
    render(<RAppLayout activeKey="health" footer={<a href="/settings">Car settings</a>}><p>Body</p></RAppLayout>);
    const page = screen.getByText('Body').parentElement!;
    expect(page.lastElementChild).toHaveTextContent('Car settings');
  });

  it('shows freshness only where a page puts it', async () => {
    const { RFreshnessIndicator } = await import('../features/r-experience/RAppLayout');
    const status = { last_updated: new Date(Date.now() - (6 * 60 + 5) * 60_000).toISOString(), telemetry_stale: true } as never;
    const { container } = render(<RFreshnessIndicator status={status} />);
    expect(screen.getByText('No data for 6 h')).toBeInTheDocument();
    expect(container.querySelector('.r-freshness')).toHaveAttribute('data-state', 'stale');
  });
});
