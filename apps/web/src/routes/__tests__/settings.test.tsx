import React from 'react';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { AuthenticationSettings, Place } from '@panda/types';

const mocks = vi.hoisted(() => ({
  role: 'super_user' as 'user' | 'super_user',
  revoke: vi.fn(),
  trigger: vi.fn(),
  backfill: { status: 'done', history_backfilled_at: null as string | null, rivian_session_count: 40, local_session_count: 37, missing_source_count: null as number | null },
  vehicleAuth: null as string | null,
  banner: null as null | { kind: 'rivian'; since: string | null; reason: 'credentials' | 'feed' },
  places: [] as Place[],
  api: {
    getOidcIdentities: vi.fn(),
    getAuthenticationSettings: vi.fn(),
    updateAuthenticationSettings: vi.fn(),
    testAuthenticationSettings: vi.fn(),
    startOidcLink: vi.fn(),
    changePassword: vi.fn(),
    updatePlace: vi.fn(),
    getBackupOverview: vi.fn(),
    getExternalConnections: vi.fn(),
    updateExternalConnection: vi.fn(),
  },
  logout: vi.fn(),
  navigate: vi.fn(),
}));

vi.mock('@tanstack/react-router', () => ({
  Link: ({ to, search, children, ...props }: { to: string; search?: Record<string, string>; children: React.ReactNode }) =>
    <a href={`${to}${search ? `?${new URLSearchParams(search)}` : ''}`} {...props}>{children}</a>,
  useNavigate: () => mocks.navigate,
  useLocation: () => ({ pathname: '/settings' }),
}));

vi.mock('../../features/r-experience/RAppLayout', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../features/r-experience/RAppLayout')>()),
  RAppLayout: ({ children }: { children: React.ReactNode }) => <main>{children}</main>,
}));

// The card reads the same connection state as the app banner.
vi.mock('../../features/r-experience/useRConnection', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../features/r-experience/useRConnection')>()),
  useRConnection: () => ({ banner: mocks.banner }),
}));

vi.mock('../../components/settings/BackupSection', () => ({ BackupSection: () => <p>Backup schedule editor</p> }));

vi.mock('@panda/hooks', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@panda/hooks')>();
  return {
    queryKeys: actual.queryKeys,
    BASEMAP_CONFIG_QUERY_KEY: actual.BASEMAP_CONFIG_QUERY_KEY,
    api: mocks.api,
    invalidateChargingData: vi.fn(),
    useAuth: () => ({ clearSession: vi.fn(), logout: mocks.logout }),
    useMe: () => ({ data: { user_id: 'u1', email: 'owner@example.test', role: mocks.role, password_configured: true } }),
    useRevokeOtherAuthSessions: () => ({ mutate: mocks.revoke, isPending: false, isError: false }),
    useChargeBackfillStatus: () => ({ data: mocks.backfill }),
    useTriggerChargeBackfill: () => ({ mutate: mocks.trigger, isPending: false, isError: false }),
    useResolvedVehicleSelection: () => ({ effectiveVehicleId: 'v1' }),
    useVehicles: () => ({ data: [{
      id: 'v1', display_name: 'Bumble Beee', membership_role: 'owner', created_at: '2026-10-06T10:00:00Z',
      credential_issued_at: '2026-10-06T10:00:00Z', auth_state: mocks.vehicleAuth,
    }] }),
    useCurrentVehicleStatus: () => ({ data: { vehicle_id: 'v1', last_updated: new Date().toISOString(), auth_state: mocks.vehicleAuth } }),
    useSavedPlaces: () => ({ data: mocks.places, isLoading: false, isError: false }),
  };
});

import { SettingsContent, rSettingsSection } from '../../features/settings/SettingsPage';
import { rSignInRules, rSignInState } from '../../features/settings/RSignInCard';
import { rParseRate, rPlaceRate, rRatePlaces, rRateUpdateBody } from '../../features/settings/RRatesCard';
import { rBackupSummary } from '../../features/settings/RBackupsCard';
import { rBackfillLine, rRivianCardState } from '../../features/settings/RRivianCard';

const place = (overrides: Partial<Place>): Place => ({
  id: 'p-home', name: 'Home', latitude: 47.6, longitude: -122.3, radius_m: 120, is_home: true, is_work: false,
  address: { display_name: '1 Main St', osm_id: 1, latitude: 47.6, longitude: -122.3, road: 'Main St', city: 'Seattle', state: 'WA', postcode: null, country: 'US', raw: null },
  charging: { id: 'cp-1', name: 'PSE Schedule 7', billing_type: 'per_kwh', rate: 0.189119, session_fee: 0, currency: 'USD', timezone: null, tou_periods: [] },
  ...overrides,
});

const authSettings = (overrides: Record<string, unknown> = {}): AuthenticationSettings => {
  const values: Record<string, unknown> = {
    oidc_enabled: false, password_login_enabled: true, oidc_auto_login: false, issuer_url: '', public_base_url: '',
    client_id: '', button_label: 'Sign in with SSO', scopes: 'openid', token_auth_method: 'auto', auto_signup: false,
    auto_link_verified_email: false, allowed_email_domains: [], required_claim_name: null, required_claim_value: null, ...overrides,
  };
  return {
    ...Object.fromEntries(Object.entries(values).map(([key, value]) => [key, { value, source: 'database' }])),
    client_secret: { configured: Boolean(overrides.issuer_url), source: 'database' },
    last_validation_at: overrides.tested ? '2026-11-01T00:00:00Z' : null,
    callback_url: 'https://r.example.test/v1/auth/oidc/callback',
  } as unknown as AuthenticationSettings;
};

function renderSettings() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(<QueryClientProvider client={client}><SettingsContent /></QueryClientProvider>);
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.role = 'super_user';
  mocks.vehicleAuth = null;
  mocks.banner = null;
  mocks.places = [place({}), place({ id: 'p-office', name: 'Office', is_home: false, is_work: true,
    charging: { id: 'cp-2', name: 'Office', billing_type: 'free', rate: 0, session_fee: 0, currency: 'USD', timezone: null, tou_periods: [] } })];
  mocks.api.getOidcIdentities.mockResolvedValue({ password_configured: true, oidc_linked: false, oidc_link_available: false });
  mocks.api.getAuthenticationSettings.mockResolvedValue(authSettings());
  mocks.api.updateAuthenticationSettings.mockImplementation(async (body) => authSettings(body));
  mocks.api.updatePlace.mockResolvedValue({});
  mocks.api.getBackupOverview.mockResolvedValue({
    settings: { enabled: true, frequency: 'daily' }, artifacts: [{ id: 'a1' }, { id: 'a2' }], recent_runs: [{ status: 'succeeded' }],
    latest_successful_run: { completed_at: '2026-11-07T11:01:00Z', created_at: '2026-11-07T11:00:00Z' },
  });
  mocks.api.getExternalConnections.mockResolvedValue({
    can_manage: true,
    connections: [
      { id: 'basemap', name: 'Map basemap', purpose: 'Street context behind routes.', enabled: true, mode: 'remote', basemap_provider: 'openfreemap', last_error: null },
      { id: 'nominatim', name: 'Place & address lookup', purpose: 'Street names for trips and charges.', enabled: false, mode: 'disabled', last_error: null },
    ],
  });
  mocks.api.updateExternalConnection.mockResolvedValue({
    can_manage: true,
    connections: [
      { id: 'basemap', name: 'Map basemap', purpose: 'Street context behind routes.', enabled: false, mode: 'disabled', basemap_provider: 'openfreemap', last_error: null },
      { id: 'nominatim', name: 'Place & address lookup', purpose: 'Street names for trips and charges.', enabled: false, mode: 'disabled', last_error: null },
    ],
  });
});

describe('Settings page', () => {
  it('shows the R cards including Maps & connected services, with the page title', async () => {
    renderSettings();
    expect(screen.getByRole('heading', { level: 1, name: 'Settings' })).toBeInTheDocument();
    for (const name of ['Account', 'Sign-in methods', 'Rivian account', 'Maps & connected services', 'Charging rates']) {
      expect(screen.getByRole('heading', { level: 2, name })).toBeInTheDocument();
    }
    expect(screen.getByRole('region', { name: 'Backups' })).toBeInTheDocument();
    for (const removed of [/units/i, /api access/i, /dashboards/i, /external/i, /places/i, /jobs/i, /raw telemetry/i, /appearance/i]) {
      expect(screen.queryByRole('heading', { name: removed })).not.toBeInTheDocument();
    }
    const mapSwitch = await screen.findByRole('switch', { name: 'Map basemap' });
    expect(mapSwitch).toHaveAttribute('aria-checked', 'true');
    fireEvent.click(mapSwitch);
    await waitFor(() => expect(mocks.api.updateExternalConnection).toHaveBeenCalledWith('basemap', expect.objectContaining({ enabled: false, mode: 'disabled' })));
  });

  it('shows the account and signs out the other devices', () => {
    renderSettings();
    const account = screen.getByRole('region', { name: 'Account' });
    expect(within(account).getByText('owner@example.test')).toBeInTheDocument();
    expect(within(account).getByText('Password')).toBeInTheDocument();
    expect(within(account).getByText('Owner')).toBeInTheDocument();
    fireEvent.click(within(account).getByRole('button', { name: 'Sign out other devices' }));
    expect(mocks.revoke).toHaveBeenCalledTimes(1);
    expect(within(account).getByRole('button', { name: /^Sign out$/ })).toBeInTheDocument();
  });

  it('changes the password and sends the owner to sign in again', async () => {
    mocks.api.changePassword.mockResolvedValue(undefined);
    renderSettings();
    fireEvent.click(screen.getByRole('button', { name: 'Change password' }));
    fireEvent.change(screen.getByLabelText('Current password'), { target: { value: 'old-password-123' } });
    fireEvent.change(screen.getByLabelText('New password'), { target: { value: 'new-password-1234' } });
    fireEvent.change(screen.getByLabelText('Confirm new password'), { target: { value: 'new-password-1234' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save password' }));
    await waitFor(() => expect(mocks.api.changePassword).toHaveBeenCalledWith({ current_password: 'old-password-123', new_password: 'new-password-1234' }));
    await waitFor(() => expect(mocks.navigate).toHaveBeenCalledWith({ to: '/login', search: { password_changed: '1' } }));
  });

  it('keeps password sign-in on until SSO is on, tested and linked', async () => {
    renderSettings();
    const password = await screen.findByRole('switch', { name: 'Password sign-in' });
    const sso = screen.getByRole('switch', { name: 'Single sign-on (SSO)' });
    expect(password).toHaveAttribute('aria-checked', 'true');
    expect(password).toHaveAttribute('aria-disabled', 'true');
    expect(sso).toHaveAttribute('aria-disabled', 'true');
    expect(screen.getByText('Turn on SSO first.')).toBeInTheDocument();
    fireEvent.click(password);
    expect(mocks.api.updateAuthenticationSettings).not.toHaveBeenCalled();
    expect(screen.getByText('Turn off password sign-in only after SSO works. If both are off, nobody can sign in.')).toBeInTheDocument();
  });

  it('turns SSO on after it is set up and tested', async () => {
    mocks.api.getAuthenticationSettings.mockResolvedValue(authSettings({ issuer_url: 'https://login.example.test', client_id: 'r', tested: true }));
    renderSettings();
    const sso = await screen.findByRole('switch', { name: 'Single sign-on (SSO)' });
    await waitFor(() => expect(sso).not.toHaveAttribute('aria-disabled'));
    fireEvent.click(sso);
    await waitFor(() => expect(mocks.api.updateAuthenticationSettings).toHaveBeenCalledWith({ oidc_enabled: true }));
  });

  it('sets up SSO, then tests the provider', async () => {
    mocks.api.testAuthenticationSettings.mockResolvedValue({ valid: true });
    renderSettings();
    fireEvent.click(await screen.findByRole('button', { name: 'Set up SSO' }));
    fireEvent.change(screen.getByLabelText('Issuer URL'), { target: { value: 'https://login.example.test' } });
    fireEvent.change(screen.getByLabelText('Client ID'), { target: { value: 'r-client' } });
    fireEvent.change(screen.getByLabelText('Client secret'), { target: { value: 'secret' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save and test connection' }));
    await waitFor(() => expect(mocks.api.testAuthenticationSettings).toHaveBeenCalled());
    expect(mocks.api.updateAuthenticationSettings).toHaveBeenCalledWith(expect.objectContaining({
      issuer_url: 'https://login.example.test', client_id: 'r-client', client_secret: 'secret',
    }));
    expect(await screen.findByText('SSO works. You can now turn it on.')).toBeInTheDocument();
  });

  it('shows the Rivian connection, reconnect link and charge recovery', () => {
    mocks.backfill = { ...mocks.backfill, missing_source_count: null };
    renderSettings();
    const rivian = screen.getByRole('region', { name: 'Rivian account' });
    expect(within(rivian).getByText('Connected')).toBeInTheDocument();
    expect(within(rivian).getByRole('link', { name: 'Reconnect' })).toHaveAttribute('href', '/connect?mode=refresh&vehicle_id=v1');
    expect(within(rivian).getByText('Rivian lists 3 charges that R does not have.')).toBeInTheDocument();
    fireEvent.click(within(rivian).getByRole('button', { name: 'Recover now' }));
    expect(mocks.trigger).toHaveBeenCalledTimes(1);
  });

  it('says when Rivian needs a reconnect', () => {
    mocks.vehicleAuth = 'needs_reauth';
    renderSettings();
    const rivian = screen.getByRole('region', { name: 'Rivian account' });
    expect(within(rivian).getByText('Reconnect needed')).toBeInTheDocument();
    expect(within(rivian).getByRole('button', { name: 'Recover now' })).toBeDisabled();
  });

  it('agrees with the app banner when no new data arrives from Rivian', () => {
    mocks.banner = { kind: 'rivian', since: null, reason: 'feed' };
    renderSettings();
    const rivian = screen.getByRole('region', { name: 'Rivian account' });
    expect(within(rivian).queryByText('Connected')).not.toBeInTheDocument();
    expect(within(rivian).getByText('Not receiving')).toBeInTheDocument();
    expect(within(rivian).getByText(/No new data is arriving\./)).toBeInTheDocument();
    expect(within(rivian).getByRole('link', { name: 'Reconnect' })).toBeInTheDocument();
  });

  it('says Reconnect needed when the banner reports expired Rivian credentials', () => {
    mocks.banner = { kind: 'rivian', since: null, reason: 'credentials' };
    renderSettings();
    const rivian = screen.getByRole('region', { name: 'Rivian account' });
    expect(within(rivian).getByText('Reconnect needed')).toBeInTheDocument();
    expect(within(rivian).getByRole('button', { name: 'Recover now' })).toBeDisabled();
  });

  it('saves the Home rate to the Home place cost profile and leaves Office alone', async () => {
    renderSettings();
    const home = screen.getByLabelText('Home · $ per kWh');
    expect(home).toHaveValue('0.189119');
    expect(screen.getByLabelText('Office · $ per kWh')).toHaveValue('0');
    fireEvent.change(home, { target: { value: '0.21' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save rates' }));
    await waitFor(() => expect(mocks.api.updatePlace).toHaveBeenCalledTimes(1));
    expect(mocks.api.updatePlace).toHaveBeenCalledWith('p-home', expect.objectContaining({
      name: 'Home', is_home: true, radius_m: 120, address: expect.objectContaining({ display_name: '1 Main St' }),
      charging: expect.objectContaining({ billing_type: 'per_kwh', rate: 0.21, name: 'PSE Schedule 7' }),
    }));
    expect(await screen.findByText('Rates saved. Charge costs update with them.')).toBeInTheDocument();
  });

  it('rejects a rate that is not a number', () => {
    renderSettings();
    fireEvent.change(screen.getByLabelText('Home · $ per kWh'), { target: { value: 'abc' } });
    expect(screen.getByText('Enter a rate such as 0.19.')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Save rates' })).toBeDisabled();
  });

  it('collapses Backups behind a summary with the backup count', async () => {
    renderSettings();
    const backups = screen.getByRole('region', { name: 'Backups' });
    expect(backups.querySelector('details')).not.toHaveAttribute('open');
    expect(await within(backups).findByText('2 daily automatic backups kept, all completed. Nothing to do here.')).toBeInTheDocument();
  });
});

describe('Settings rules', () => {
  const base = { passwordOn: true, ssoOn: false, ssoConfigured: false, ssoTested: false, accountLinked: false };
  it('never allows both sign-in methods off', () => {
    expect(rSignInRules({ ...base, ssoOn: true, ssoTested: true, accountLinked: true, passwordOn: false }).canTurnSsoOff).toBe(false);
    expect(rSignInRules({ ...base }).canTurnPasswordOff).toBe(false);
  });
  it('turns password sign-in off only after SSO works for this account', () => {
    expect(rSignInRules({ ...base, ssoOn: true, ssoTested: false, accountLinked: true }).passwordOffReason).toBe('Test SSO first.');
    expect(rSignInRules({ ...base, ssoOn: true, ssoTested: true, accountLinked: false }).passwordOffReason).toBe('Connect SSO to this account first.');
    expect(rSignInRules({ ...base, ssoOn: true, ssoTested: true, accountLinked: true }).canTurnPasswordOff).toBe(true);
  });
  it('turns SSO on only after it is set up and tested', () => {
    expect(rSignInRules({ ...base, ssoConfigured: true }).canTurnSsoOn).toBe(false);
    expect(rSignInRules({ ...base, ssoConfigured: true, ssoTested: true }).canTurnSsoOn).toBe(true);
    expect(rSignInState(authSettings({ issuer_url: 'https://x', client_id: 'c', tested: true }), false)).toMatchObject({ ssoConfigured: true, ssoTested: true });
  });
  it('maps old section links to the new cards', () => {
    expect(rSettingsSection('authentication')).toBe('sign-in');
    expect(rSettingsSection('vehicles')).toBe('rivian');
    expect(rSettingsSection('places')).toBe('rates');
    expect(rSettingsSection('backup')).toBe('backups');
    expect(rSettingsSection('units')).toBeUndefined();
  });
  it('reads, parses and writes rates', () => {
    const targets = rRatePlaces([place({}), place({ id: 'w', name: 'Work', is_home: false, is_work: false, charging: null })]);
    expect(targets.home?.id).toBe('p-home');
    expect(targets.office?.id).toBe('w');
    expect(rPlaceRate(targets.office)).toBeNull();
    expect(rParseRate('$0.19')).toBe(0.19);
    expect(rParseRate('0,25')).toBe(0.25);
    expect(rParseRate('-1')).toBeNull();
    const noAddress = rRateUpdateBody(place({ address: null }), 0);
    expect(noAddress.address).toMatchObject({ latitude: 47.6, longitude: -122.3, display_name: 'Home' });
    expect(noAddress.charging).toMatchObject({ billing_type: 'per_kwh', rate: 0 });
  });
  it('words the backfill and backup states', () => {
    expect(rBackfillLine(undefined, true)).toBe('Fetching charges from Rivian…');
    expect(rBackfillLine({ vehicle_id: 'v', status: 'error', history_backfilled_at: null, rivian_session_count: null, local_session_count: 0 }, false)).toBe('The last recovery failed. Try again.');
    expect(rBackupSummary(undefined).note).toBe('Backup status is not available.');
    expect(rRivianCardState(null, false)).toBe('connected');
    expect(rRivianCardState(null, true)).toBe('reconnect');
    expect(rRivianCardState({ kind: 'offline', since: null }, false)).toBe('connected');
    expect(rRivianCardState({ kind: 'rivian', since: null, reason: 'feed' }, false)).toBe('feed');
    expect(rRivianCardState({ kind: 'rivian', since: null, reason: 'credentials' }, false)).toBe('reconnect');
  });
});
