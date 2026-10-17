import React from 'react';
import { Link, useLocation, useNavigate } from '@tanstack/react-router';
import { Battery, CarFront, Compass, Gauge, LogOut, Moon, Route, Settings2, Sun, Zap } from 'lucide-react';
import { useAuth } from '@panda/hooks';
import { useDocumentTheme } from '@panda/ui/hooks';
import { useThemePreferenceController } from '../../hooks/useThemePreferenceController';
import { RMark } from './RMark';
import { useRConnection } from './useRConnection';
import { mediaStore } from './browserBindings';
import { primaryNavigation } from './navigationModel';

const navigation = [
  { key: 'dashboard', label: 'Home', to: '/', icon: CarFront },
  { key: 'trips', label: 'Trips', to: '/trips', icon: Route },
  { key: 'charging', label: 'Charging', to: '/charging', icon: Zap },
  { key: 'battery', label: 'Battery', to: '/battery', icon: Battery },
  { key: 'efficiency', label: 'Efficiency', to: '/efficiency', icon: Gauge },
  { key: 'explore', label: 'Explore', to: '/explore', icon: Compass },
] as const;

const navigationViewport = mediaStore('(max-width: 900px)');
const compactDestinations = navigation.filter(({ key }) => key !== 'efficiency');
const themeControls = new Map([
  [false, { mode: 'dark' as const, icon: Moon }],
  [true, { mode: 'light' as const, icon: Sun }],
]);

export function RAppLayout({ children, activeKey }: { children: React.ReactNode; activeKey: string }) {
  const { pathname } = useLocation();
  const navigate = useNavigate();
  const logout = useAuth(state => state.logout);
  const theme = useThemePreferenceController();
  const dark = useDocumentTheme();
  const connection = useRConnection();
  const compact = React.useSyncExternalStore(navigationViewport.subscribe, navigationViewport.snapshot, navigationViewport.serverSnapshot);
  const destinations = primaryNavigation(activeKey, compact ? compactDestinations : navigation);
  const { mode: nextMode, icon: ThemeIcon } = themeControls.get(dark)!;
  const switchTheme = () => theme.onModeChange(nextMode);
  const signOut = () => logout().then(() => navigate({ to: '/login' }));
  return (
    <div className="r-app" data-area={activeKey}>
      <a className="r-skip-link" href="#r-content">Skip to content</a>
      <header className="r-header">
        <Link to="/" aria-label="R:The Drive overview" className="r-brand"><RMark /></Link>
        <nav className="r-navigation" aria-label="Primary navigation">
          {destinations.map(({ key, label, to, current, icon: Icon }) => (
            <Link key={key} to={to} aria-label={label} aria-current={current}>
              <Icon aria-hidden="true" /><span>{label}</span>
            </Link>
          ))}
        </nav>
        <div className="r-header-actions">
          <button type="button" aria-label={`Switch to ${nextMode} mode`}
            disabled={theme.isPending} onClick={switchTheme}>
            <ThemeIcon aria-hidden="true" />
          </button>
          <Link to="/settings" aria-label="Settings"><Settings2 aria-hidden="true" /></Link>
          <button type="button" aria-label="Sign out" onClick={signOut}><LogOut aria-hidden="true" /></button>
        </div>
      </header>
      <main id="r-content" className="r-main" tabIndex={-1}>
        {connection.onlineState === 'connecting' && <p className="r-connection-status" role="status">
          Reconnecting live updates · Showing last recorded readings.
        </p>}
        {connection.feed && <p className="r-notice" role="alert">{connection.feed.title}.{' '}
          <Link to="/vehicle-health">Check vehicle health</Link></p>}
        {connection.renewal && <p className="r-notice">
          <Link to="/settings" search={{ section: 'vehicles' }}>{connection.renewal.message}</Link>
        </p>}
        <div className="r-page" key={pathname}>{children}</div>
      </main>
    </div>
  );
}
