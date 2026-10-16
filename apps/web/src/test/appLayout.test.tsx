import type { AnchorHTMLAttributes } from 'react';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { AppLayout } from '../components/layout/AppLayout';

const state = vi.hoisted(() => ({
  navigate: vi.fn(),
  logout: vi.fn(async () => {}),
  onModeChange: vi.fn(),
  dark: false,
  pending: false,
  connection: {
    onlineState: 'online',
    feed: null as { title: string } | null,
    renewal: null as { message: string } | null,
  },
}));

vi.mock('@tanstack/react-router', () => ({
  useNavigate: () => state.navigate,
  useLocation: () => ({ pathname: '/' }),
  Link: ({
    to, search, ...props
  }: AnchorHTMLAttributes<HTMLAnchorElement> & { to: string; search?: Record<string, string> }) =>
    <a href={`${to}${search ? `?${new URLSearchParams(search)}` : ''}`} {...props} />,
}));

vi.mock('@panda/hooks', () => ({
  useAuth: (selector: (value: { logout: typeof state.logout }) => unknown) =>
    selector({ logout: state.logout }),
}));

vi.mock('@panda/ui/hooks', () => ({ useDocumentTheme: () => state.dark }));
vi.mock('../hooks/useThemePreferenceController', () => ({
  useThemePreferenceController: () => ({
    isPending: state.pending,
    onModeChange: state.onModeChange,
  }),
}));
vi.mock('../features/r-experience/useRConnection', () => ({
  useRConnection: () => state.connection,
}));

describe('R application layout', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    state.dark = false;
    state.pending = false;
    state.connection = { onlineState: 'online', feed: null, renewal: null };
    vi.spyOn(window, 'matchMedia').mockImplementation(query => ({
      matches: false, media: query, onchange: null,
      addListener: vi.fn(), removeListener: vi.fn(),
      addEventListener: vi.fn(), removeEventListener: vi.fn(),
      dispatchEvent: vi.fn(),
    }));
  });

  it('renders the desktop R destinations and accessible content through the shared layout entry', () => {
    render(<AppLayout activeKey="trips"><h1>Trip history</h1></AppLayout>);
    const navigation = within(screen.getByRole('navigation', { name: 'Primary navigation' }));
    expect(navigation.getAllByRole('link').map(link => link.textContent)).toEqual([
      'Home', 'Trips', 'Charging', 'Battery', 'Efficiency', 'Explore',
    ]);
    expect(navigation.getByRole('link', { name: 'Trips' })).toHaveAttribute('aria-current', 'page');
    expect(screen.getByRole('link', { name: 'Settings' })).toHaveAttribute('href', '/settings');
    expect(screen.getByRole('link', { name: 'Skip to content' })).toHaveAttribute('href', '#r-content');
    expect(screen.getByRole('main')).toContainElement(screen.getByRole('heading', { name: 'Trip history' }));
  });

  it('keeps five mobile destinations with efficiency under Explore', () => {
    vi.mocked(window.matchMedia).mockImplementation(query => ({
      matches: query === '(max-width: 900px)', media: query, onchange: null,
      addListener: vi.fn(), removeListener: vi.fn(),
      addEventListener: vi.fn(), removeEventListener: vi.fn(),
      dispatchEvent: vi.fn(),
    }));
    render(<AppLayout activeKey="efficiency">Efficiency</AppLayout>);
    const navigation = within(screen.getByRole('navigation', { name: 'Primary navigation' }));
    expect(navigation.getAllByRole('link')).toHaveLength(5);
    expect(navigation.queryByRole('link', { name: 'Efficiency' })).not.toBeInTheDocument();
    expect(navigation.getByRole('link', { name: 'Explore' })).toHaveAttribute('aria-current', 'page');
  });

  it('keeps Battery selected on its detail pages', () => {
    render(<AppLayout activeKey="battery.phantom-drain">Parked history</AppLayout>);
    expect(screen.getByRole('link', { name: 'Battery' })).toHaveAttribute('aria-current', 'page');
  });

  it('uses account theme preferences for the existing light/dark control', () => {
    const view = render(<AppLayout activeKey="dashboard">Home</AppLayout>);
    fireEvent.click(screen.getByRole('button', { name: 'Switch to dark mode' }));
    expect(state.onModeChange).toHaveBeenCalledWith('dark');
    state.dark = true;
    state.pending = true;
    view.rerender(<AppLayout activeKey="dashboard">Home</AppLayout>);
    expect(screen.getByRole('button', { name: 'Switch to light mode' })).toBeDisabled();
  });

  it('waits for logout before navigating to sign-in', async () => {
    let finish!: () => void;
    state.logout.mockImplementationOnce(() => new Promise<void>(resolve => { finish = resolve; }));
    render(<AppLayout activeKey="dashboard">Home</AppLayout>);
    fireEvent.click(screen.getByRole('button', { name: 'Sign out' }));
    expect(state.navigate).not.toHaveBeenCalled();
    finish();
    await waitFor(() => expect(state.navigate).toHaveBeenCalledWith({ to: '/login' }));
  });

  it('preserves reconnecting and feed-health notices', () => {
    state.connection.onlineState = 'connecting';
    state.connection.feed = { title: 'Telemetry needs attention' };
    render(<AppLayout activeKey="dashboard">Home</AppLayout>);
    expect(screen.getByRole('status')).toHaveTextContent('Showing last recorded readings');
    expect(screen.getByRole('alert')).toHaveTextContent('Telemetry needs attention');
    expect(screen.getByRole('link', { name: 'Check vehicle health' })).toHaveAttribute('href', '/vehicle-health');
  });

  it('takes credential renewal to vehicle settings', () => {
    state.connection.renewal = { message: 'Renew your vehicle connection' };
    render(<AppLayout activeKey="dashboard">Home</AppLayout>);
    expect(screen.getByRole('link', { name: 'Renew your vehicle connection' }))
      .toHaveAttribute('href', '/settings?section=vehicles');
  });
});
