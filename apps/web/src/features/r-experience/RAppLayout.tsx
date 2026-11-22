import React from 'react';
import { Link, useLocation } from '@tanstack/react-router';
import type { VehicleStatus } from '@panda/types';
import { useThemeRuntime } from '@panda/ui/lib/theme';
import { formatAppDate, formatAppTime } from '@panda/ui/lib/dateTime';
import { useThemePreferenceController } from '../../hooks/useThemePreferenceController';
import { RMark } from './RMark';
import { useRConnection, type RConnectionBanner } from './useRConnection';
import {
  RIconActivity, RIconCar, RIconGear, RIconInsights, RIconMoon, RIconNow, RIconOffline, RIconSun, RIconWarn,
} from './RShellIcons';

export type RTabKey = 'now' | 'activity' | 'insights' | 'car';
export type RAreaKey = RTabKey | 'settings';

/** The four tabs. Settings sits at the bottom of the desktop rail and behind the gear on phones. */
export const rNavigation = [
  { key: 'now', label: 'Now', to: '/', icon: RIconNow },
  { key: 'activity', label: 'Activity', to: '/trips', icon: RIconActivity },
  { key: 'insights', label: 'Insights', to: '/insights', icon: RIconInsights },
  { key: 'car', label: 'Car', to: '/vehicle-health', icon: RIconCar },
] as const satisfies ReadonlyArray<{ key: RTabKey; label: string; to: string; icon: React.ElementType }>;

const AREA_TITLES: Record<RAreaKey, string> = { now: 'Now', activity: 'Activity', insights: 'Insights', car: 'Car', settings: 'Settings' };

const within = (pathname: string, base: string) => pathname === base || pathname.startsWith(`${base}/`);

/** Maps a URL to its area. Detail pages keep the tab of the list they open from. */
export function rNavigationTab(pathname: string): RAreaKey {
  const path = pathname.length > 1 ? pathname.replace(/\/+$/, '') : pathname;
  if (path === '/' || path === '') return 'now';
  if (within(path, '/trips') || /^\/charging\/[^/]+$/.test(path)) return 'activity';
  if (['/insights', '/charging', '/battery', '/efficiency'].some(base => within(path, base))) return 'insights';
  if (['/settings', '/connect', '/users'].some(base => within(path, base))) return 'settings';
  return 'car';
}

export interface RFreshness {
  state: 'live' | 'stale';
  label: string;
  at: string;
}

function age(minutes: number) {
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  return hours < 48 ? `${hours} h` : `${Math.floor(hours / 24)} days`;
}

/** Freshness of the latest vehicle status, or how long no data came in. Pages show it in their header. */
export function rFreshness(
  status: Pick<VehicleStatus, 'last_updated' | 'telemetry_stale'> | null | undefined,
  now = Date.now(),
): RFreshness | null {
  const at = status?.last_updated;
  const time = at ? new Date(at).getTime() : Number.NaN;
  if (!at || !Number.isFinite(time)) return null;
  const minutes = Math.max(0, Math.floor((now - time) / 60_000));
  if (status?.telemetry_stale) return { state: 'stale', label: `No data for ${age(Math.max(1, minutes))}`, at };
  return { state: 'live', label: minutes < 1 ? 'Updated just now' : `Updated ${age(minutes)} ago`, at };
}

function useMinuteClock() {
  const [now, setNow] = React.useState(() => Date.now());
  React.useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 30_000);
    return () => window.clearInterval(timer);
  }, []);
  return now;
}

/** "Updated 5 min ago" or "No data for 6 h". Put it in a page header sub line. */
export function RFreshnessIndicator({ status }: { status: VehicleStatus | null }) {
  const freshness = rFreshness(status, useMinuteClock());
  if (!freshness) return null;
  return (
    <span className="r-freshness" data-state={freshness.state}>
      <span className="r-freshness-dot" aria-hidden="true" />
      <time dateTime={freshness.at}>{freshness.label}</time>
    </span>
  );
}

/** A time today, or a date and time for an older reading, in the app time zone. */
export function rWhen(at: string | null | undefined, now = Date.now()): string | null {
  if (!at) return null;
  const time = new Date(at).getTime();
  if (!Number.isFinite(time)) return null;
  const sameDay = formatAppDate(time) === formatAppDate(now);
  return sameDay ? formatAppTime(time) : `${formatAppDate(time, { year: undefined })}, ${formatAppTime(time)}`;
}

/** Light and dark switch. Saves the choice to the account like the Settings appearance control. */
export function RThemeButton() {
  const theme = useThemePreferenceController();
  const dark = useThemeRuntime().effectiveMode !== 'light';
  return (
    <button type="button" className="r-iconbtn" aria-label={`Switch to ${dark ? 'light' : 'dark'} mode`}
      disabled={theme.isPending} onClick={() => theme.onModeChange(dark ? 'light' : 'dark')}>
      {dark ? <RIconSun /> : <RIconMoon />}
    </button>
  );
}

function RTopActions({ area }: { area: RAreaKey }) {
  return (
    <div className="r-top-actions">
      {area !== 'settings' && <Link to="/settings" className="r-iconbtn r-gearbtn" aria-label="Settings"><RIconGear /></Link>}
      <RThemeButton />
    </div>
  );
}

const HeaderSlot = React.createContext<{ area: RAreaKey; claim: () => () => void } | null>(null);

/**
 * The gear and theme buttons inside a page's own title row. A page that renders it takes the
 * buttons out of the layout's separate row, so they stay beside the title when a banner shows.
 */
export function RPageActions() {
  const slot = React.useContext(HeaderSlot);
  React.useLayoutEffect(() => slot?.claim(), [slot]);
  return slot ? <RTopActions area={slot.area} /> : null;
}

/** For pages with their own back control (phone detail pages): no gear and theme row. */
export function useHidePageActions() {
  const slot = React.useContext(HeaderSlot);
  React.useLayoutEffect(() => slot?.claim(), [slot]);
}

/**
 * Page header from the trial (`top()`): title, optional sub line, and the gear and theme buttons.
 * Use it as the first element of a page. Without it, the layout shows the buttons at the top right.
 */
export function RPageHeader({ title, sub, extra }: { title: React.ReactNode; sub?: React.ReactNode; extra?: React.ReactNode }) {
  const slot = React.useContext(HeaderSlot);
  React.useLayoutEffect(() => slot?.claim(), [slot]);
  return (
    <header className="r-top">
      <div className="r-top-text">
        <h1>{title}</h1>
        {(sub || extra) && <div className="r-top-sub">{extra}{sub}</div>}
      </div>
      {slot && <RTopActions area={slot.area} />}
    </header>
  );
}

/** Offline and Rivian connection banners, from the browser state and the vehicle status. */
export function RConnectionBannerView({ banner, vehicleId }: { banner: RConnectionBanner | null; vehicleId: string | null }) {
  if (!banner) return null;
  const since = rWhen(banner.since);
  if (banner.kind === 'offline') {
    return (
      <div className="r-banner r-banner-offline" role="status">
        <RIconOffline />
        <span><b>You&apos;re offline.</b> {since ? `Showing data from ${since}. ` : 'Showing the last data R loaded. '}It updates when you reconnect.</span>
      </div>
    );
  }
  return (
    <div className="r-banner r-banner-error" role="alert">
      <RIconWarn />
      <span>
        <b>Rivian connection lost.</b> {since ? `No new data since ${since}. ` : 'No new data is arriving. '}
        New trips and charges are missing. After you reconnect, R recovers the missed charges from Rivian.
      </span>
      {vehicleId && <Link className="r-btn r-btn-sm" to="/connect" search={{ mode: 'refresh', vehicle_id: vehicleId }}>Reconnect</Link>}
    </div>
  );
}

function useTabInk(active: RAreaKey) {
  const navRef = React.useRef<HTMLElement>(null);
  const [ink, setInk] = React.useState<{ x: number; shown: boolean }>({ x: 0, shown: false });
  React.useLayoutEffect(() => {
    const place = () => {
      const nav = navRef.current;
      const link = nav?.querySelector<HTMLElement>('[aria-current="page"]');
      if (!nav || !link || !link.offsetParent) { setInk(current => current.shown ? { ...current, shown: false } : current); return; }
      const box = link.getBoundingClientRect(), parent = nav.getBoundingClientRect();
      setInk({ x: box.left - parent.left + box.width / 2 - 14, shown: true });
    };
    place();
    window.addEventListener('resize', place);
    return () => window.removeEventListener('resize', place);
  }, [active]);
  return { navRef, ink };
}

export function RAppLayout({ children, activeKey, footer }: {
  children: React.ReactNode;
  activeKey: string;
  /** Optional page-level content after the page body. */
  footer?: React.ReactNode;
}) {
  const { pathname } = useLocation();
  const connection = useRConnection();
  const active = rNavigationTab(pathname);
  const { navRef, ink } = useTabInk(active);
  const [headers, setHeaders] = React.useState(0);
  const slot = React.useMemo(() => ({
    area: active,
    claim: () => { setHeaders(count => count + 1); return () => setHeaders(count => count - 1); },
  }), [active]);
  React.useEffect(() => { document.title = `${AREA_TITLES[active]} · R`; }, [active]);
  return (
    <div className="r-app" data-area={activeKey}>
      <a className="r-skip-link" href="#r-content">Skip to content</a>
      <div className="r-rail">
        <Link to="/" aria-label="R, Now" className="r-brand"><RMark size={36} /></Link>
        <nav className="r-navigation" aria-label="Primary navigation" ref={navRef}>
          {rNavigation.map(({ key, label, to, icon: Icon }) => (
            <Link key={key} to={to} className="r-tab" aria-label={label} aria-current={active === key ? 'page' : undefined}>
              <Icon /><span>{label}</span>
            </Link>
          ))}
          <span className="r-tab-ink" aria-hidden="true" style={{ transform: `translateX(${ink.x}px)`, opacity: ink.shown ? 1 : 0 }} />
        </nav>
        <Link to="/settings" className="r-tab r-rail-settings" aria-label="Settings" aria-current={active === 'settings' ? 'page' : undefined}>
          <RIconGear /><span>Settings</span>
        </Link>
      </div>
      <main id="r-content" className="r-main" tabIndex={-1}>
        <RConnectionBannerView banner={connection.banner} vehicleId={connection.vehicleId} />
        {connection.onlineState === 'connecting' && !connection.banner && <p className="r-connection-status" role="status">
          Reconnecting live updates · Showing last recorded readings.
        </p>}
        {connection.renewal && !connection.banner && <p className="r-notice">
          <Link to="/settings">{connection.renewal.message}</Link>
        </p>}
        {headers === 0 && <RTopActions area={active} />}
        <HeaderSlot.Provider value={slot}>
          <div className="r-page" key={pathname}>{children}{footer}</div>
        </HeaderSlot.Provider>
      </main>
    </div>
  );
}
