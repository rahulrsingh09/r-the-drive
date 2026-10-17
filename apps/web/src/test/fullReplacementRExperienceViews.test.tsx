import React from 'react';
import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { DashboardPageShellRenderState } from '../components/dashboard/dashboardShellTypes';
import type { Vehicle, VehicleStatus } from '@panda/types';
import { snapshotDom } from './fullReplacementRExperienceDom';

const fixture = vi.hoisted(() => ({
  vehicle: { id: 'synthetic-vehicle', model: 'R1T', trim: 'Adventure', color: 'Blue', display_name: 'Test vehicle', images: null },
  snapshot: { data: null as unknown, isPlaceholderData: false, isError: false, refetch: vi.fn() },
  batch: { data: { values: [{ metric: 'total_trips', value: 12 }, { metric: 'trip_miles', value: 125 }] }, isPlaceholderData: false, isError: false, refetch: vi.fn() },
  trips: { data: { items: [] as object[] }, isPlaceholderData: false, isLoading: false, isError: false, refetch: vi.fn() },
  charges: { data: { items: [] as object[] }, isPlaceholderData: false, isLoading: false, isError: false, refetch: vi.fn() },
  health: { data: null as unknown, isPlaceholderData: false },
  role: 'viewer',
  pathname: '/',
  navigate: vi.fn(),
  create: { isPending: false, mutateAsync: vi.fn() },
  clone: { isPending: false, mutateAsync: vi.fn() },
  parse: vi.fn(),
  download: vi.fn(),
  basemap: 'openfreemap',
}));

vi.mock('@tanstack/react-router', () => ({
  Link: ({ to, params, search, activeOptions, children, ...props }: React.PropsWithChildren<Record<string, unknown>>) =>
    <a {...props} href={String(to)} data-params={params ? JSON.stringify(params) : undefined} data-search={search ? JSON.stringify(search) : undefined} data-exact={activeOptions ? JSON.stringify(activeOptions) : undefined}>{children}</a>,
  useNavigate: () => fixture.navigate,
  useLocation: () => ({ pathname: fixture.pathname }),
}));
vi.mock('@panda/hooks', () => ({
  useCurrentVehicleStatus: () => fixture.snapshot,
  useMetricBatch: () => fixture.batch,
  useResolvedVehicleSelection: () => ({ vehicles: [fixture.vehicle], effectiveVehicleId: fixture.vehicle.id }),
  useVehicleHealth: () => fixture.health,
  useTrips: () => fixture.trips,
  useChargeSessions: () => fixture.charges,
  useBasemapConfig: () => ({ data: { resolved_provider: fixture.basemap } }),
  useMe: () => ({ data: { role: fixture.role } }),
  resolveVehicleArtwork: () => ({ light: '/light.webp', dark: '/dark.webp', fallback: '/fallback.webp' }),
  AuthenticatedVehicleArtwork: ({ source, fallbackSource, alt }: { source: string; fallbackSource: string; alt: string }) => <img src={source} data-fallback={fallbackSource} alt={alt} />,
}));
vi.mock('@panda/dashboards', async importOriginal => ({
  ...await importOriginal<typeof import('@panda/dashboards')>(),
  DashboardRenderer: ({ config }: { config: { widgets: unknown[] } }) => <div data-widgets={JSON.stringify(config.widgets)} />,
  useCreateDashboard: () => fixture.create,
  useCloneDashboard: () => fixture.clone,
  importDashboardYaml: (text: string) => fixture.parse(text),
  materializeUserDashboardDraft: (value: unknown) => value,
  downloadDashboardYaml: (value: unknown) => fixture.download(value),
}));
vi.mock('@panda/ui/hooks', () => ({ useDocumentTheme: () => false }));
vi.mock('../features/r-experience/RAppLayout', () => ({ RAppLayout: ({ children }: React.PropsWithChildren) => <main>{children}</main> }));
vi.mock('../features/settings/AppearanceSettings', () => ({ AppearanceSettings: ({ mapStyle }: { mapStyle: string }) => <div data-map-style={mapStyle} /> }));
vi.mock('@panda/ui/primitives', () => ({
  ResponsiveDialog: ({ children, titleId, onClose }: React.PropsWithChildren<{ titleId: string; onClose: () => void }>) => <div role="dialog" aria-labelledby={titleId}>{children}<button onClick={onClose}>Close dialog</button></div>,
  SelectPicker: ({ options, value, onChange, ...props }: { options: { value: string; label: string }[]; value: string; onChange: (value: string) => void }) =>
    <select {...props} value={value} onChange={event => onChange(event.target.value)}>{options.map(option => <option key={option.value} value={option.value}>{option.label}</option>)}</select>,
}));

import { getDefaultBySlug } from '@panda/dashboards';
import { RSettingsNavigation } from '../features/r-experience/RSettingsNavigation';
import { RHomeWorkspace } from '../features/r-experience/RHomeWorkspace';
import { RDashboardVehicle, HomeVehicleHeadingContext } from '../features/r-experience/RDashboardVehicle';
import { RDashboardTools } from '../features/r-experience/RDashboardTools';
import { RDashboardSurface } from '../features/r-experience/RDashboardSurface';
import { RVehicleDetails } from '../features/r-experience/RVehicleDetails';
import { RVehicleStage } from '../features/r-experience/RVehicleStage';
import { RRecentActivity } from '../features/r-experience/RRecentActivity';
import { RVehicleLockStatus } from '../features/r-experience/RVehicleLockStatus';
import { RHealthHero } from '../features/r-experience/RHealthHero';
import { RConfiguredVehicleArtwork } from '../features/r-experience/RConfiguredVehicleArtwork';
import { RExplore } from '../features/r-experience/RExplore';
import { RBatteryNavigation } from '../features/r-experience/RBatteryNavigation';
import { RMapPreferences } from '../features/r-experience/RMapPreferences';
import { RLockIcon } from '../features/r-experience/RLockIcon';

afterEach(() => { cleanup(); vi.clearAllMocks(); });
beforeEach(() => {
  fixture.snapshot.data = null;
  fixture.snapshot.isPlaceholderData = false;
  fixture.snapshot.isError = false;
  fixture.batch.isPlaceholderData = false;
  fixture.trips.data.items = [];
  fixture.charges.data.items = [];
  fixture.trips.isLoading = fixture.trips.isError = fixture.trips.isPlaceholderData = false;
  fixture.charges.isLoading = fixture.charges.isError = fixture.charges.isPlaceholderData = false;
  fixture.create.isPending = false;
  fixture.health.data = null;
  fixture.health.isPlaceholderData = false;
  fixture.pathname = '/';
});
let captureIndex = 0;
function capture(container: HTMLElement) {
  if (process.env.R_EXPERIENCE_CAPTURE) {
    mkdirSync(process.env.R_EXPERIENCE_CAPTURE, { recursive: true });
    writeFileSync(join(process.env.R_EXPERIENCE_CAPTURE, `view-${++captureIndex}.html`), container.innerHTML);
  }
  expect(snapshotDom(container)).toMatchSnapshot();
}
const vehicle = fixture.vehicle as unknown as Vehicle;
const state = () => ({
  activeConfig: getDefaultBySlug('dashboard'),
  timeframe: '7d',
  ctx: { vehicleId: vehicle.id },
  isEditMode: false,
}) as unknown as DashboardPageShellRenderState;

it('characterizes settings grouping, search, unknown sections, and directory return', () => {
  const select = vi.fn();
  const Icon = () => <svg />;
  const sections = ['vehicles', 'units', 'account', 'raw', 'other'].map(id => ({ id, label: id, icon: Icon }));
  const view = render(<RSettingsNavigation sections={sections} active="directory" onSelect={select} />);
  capture(view.container);
  fireEvent.change(screen.getByRole('searchbox'), { target: { value: ' measurements ' } });
  expect(screen.getByRole('button', { name: /Units & time/ })).toBeInTheDocument();
  expect(screen.queryByRole('button', { name: /vehicles/ })).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: /Units & time/ }));
  expect(select).toHaveBeenLastCalledWith('units');
  fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'nothing' } });
  expect(screen.getByRole('status')).toHaveTextContent('No settings match “nothing”.');
  view.rerender(<RSettingsNavigation sections={sections} active="units" onSelect={select} />);
  capture(view.container);
  fireEvent.click(screen.getByText('All settings'));
  expect(select).toHaveBeenLastCalledWith('directory');
});

it('characterizes workspace partitioning and rebases section rows without mutating configuration', () => {
  const current = state();
  const before = JSON.stringify(current.activeConfig);
  const view = render(<RHomeWorkspace state={current} actions={<button>Actions</button>} activity={<p>Activity</p>} />);
  capture(view.container);
  expect(JSON.stringify(current.activeConfig)).toBe(before);
  view.rerender(<RHomeWorkspace state={{ ...current, activeConfig: undefined }} actions={null} activity={null} />);
  expect(view.container).toBeEmptyDOMElement();
});

it.each([null, {
  vehicle_id: vehicle.id, battery_level: 61, battery_limit: 80, range_miles: 155,
  charger_status: 'chrgr_sts_connected_charging', time_to_end_of_charge_min: 90,
  tire_fl_psi: 42, tire_fr_psi: 41, tire_rl_psi: 42, tire_rr_psi: 43,
  drive_mode: 'all_purpose', ota_current_version: '1.0', ota_available_version: '2.0',
  doors_locked: false, power_state: 'ready',
}])('preserves vehicle display markup for %j', reading => {
  const view = render(<>
    <RVehicleStage vehicle={vehicle} status={reading as VehicleStatus | null} distance={125} trips={12} efficiency={330} />
    <RVehicleDetails vehicle={vehicle} status={reading as VehicleStatus | null} />
    <RVehicleLockStatus status={reading as VehicleStatus | null} />
  </>);
  capture(view.container);
});

it('keeps artwork, map preference, and route eligibility exact', () => {
  const view = render(<>
    <RHealthHero vehicle={vehicle} name="Test" model="" vin="SYNTHETIC" source="/art.webp" fallback="/fallback.webp">Summary</RHealthHero>
    <RConfiguredVehicleArtwork vehicle={{ ...vehicle, model: 'R2', trim: 'Performance' }} className="art">fallback</RConfiguredVehicleArtwork>
    <RBatteryNavigation pathname="/battery" />
    <RBatteryNavigation pathname="/battery/phantom-drain/" />
    <RMapPreferences preferences={undefined} />
  </>);
  capture(view.container);
  fixture.basemap = 'other';
  view.rerender(<RMapPreferences preferences={undefined} />);
  expect(view.container).toBeEmptyDOMElement();
  fixture.basemap = 'openfreemap';
});

it('shows people access only for the two privileged roles', () => {
  for (const role of ['viewer', 'admin', 'super_user', 'user']) {
    fixture.role = role;
    const view = render(<RExplore />);
    expect(Boolean(screen.queryByText('People & access'))).toBe(['admin', 'super_user'].includes(role));
    if (role === 'admin') capture(view.container);
    view.unmount();
  }
});

it('keeps trip loading priority and charge error priority, suppressing placeholder values', () => {
  fixture.trips.isError = fixture.trips.isLoading = true;
  fixture.charges.isError = fixture.charges.isLoading = true;
  const view = render(<RRecentActivity ctx={{ vehicleId: vehicle.id, from: null, to: null }} />);
  expect(screen.getByText('Loading trips…')).toBeInTheDocument();
  expect(screen.getByRole('alert')).toHaveTextContent('Charging history could not be loaded.');
  fireEvent.click(screen.getByText('Retry'));
  expect(fixture.charges.refetch).toHaveBeenCalledOnce();
  capture(view.container);
  fixture.trips.isError = fixture.trips.isLoading = false;
  fixture.charges.isError = fixture.charges.isLoading = false;
  fixture.trips.data.items = [{ id: 'trip', started_at: '2026-01-02T12:00:00Z', start_place: 'A', end_address: 'B', distance_mi: 25, duration_min: 30, efficiency_wh_mi: 310 }];
  fixture.charges.data.items = [{ id: 'charge', started_at: '2026-01-02T12:00:00Z', energy_added_kwh: 10, cost_usd: 3 }];
  view.rerender(<RRecentActivity ctx={{ vehicleId: vehicle.id, from: null, to: null }} />);
  capture(view.container);
});

it('gates dashboard stage, totals and snapshot values by identity and placeholder status', () => {
  const config = state().activeConfig!;
  const instance = config.widgets[0]!;
  const view = render(<RDashboardVehicle instance={instance} ctx={{ vehicleId: vehicle.id, from: null, to: null }}>fallback</RDashboardVehicle>);
  expect(screen.getByText('fallback')).toBeInTheDocument();
  fixture.snapshot.isError = true;
  fixture.snapshot.data = { vehicle_id: 'other', battery_level: 90 };
  view.rerender(<HomeVehicleHeadingContext.Provider value={instance.id}><RDashboardVehicle instance={instance} ctx={{ vehicleId: vehicle.id, from: null, to: null }}>fallback</RDashboardVehicle></HomeVehicleHeadingContext.Provider>);
  expect(screen.queryByText('90%')).toBeNull();
  expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent('Test vehicle');
  fireEvent.click(screen.getByText('Retry'));
  expect(fixture.snapshot.refetch).toHaveBeenCalledOnce();
});


it('exports active dashboards', () => {
  const current = state();
  render(<RDashboardTools state={current} />);
  fireEvent.click(screen.getByText('Export dashboard'));
  expect(fixture.download).toHaveBeenCalledWith(current.activeConfig);

});

it('keeps home readiness and the bundled surface flag', () => {
  const current = state();
  const view = render(<RDashboardSurface state={current} actions={null} ready={false}>Grid</RDashboardSurface>);
  expect(screen.getByText('Grid')).toBeInTheDocument();
  expect(view.container.querySelector('[data-r-default-layout]')).toHaveAttribute('data-r-default-layout', 'true');
  view.rerender(<RDashboardSurface state={current} actions={null} ready>Grid</RDashboardSurface>);
  expect(screen.queryByText('Grid')).toBeNull();
  fixture.pathname = '/battery';
  view.rerender(<RDashboardSurface state={current} actions={null} ready>Grid</RDashboardSurface>);
  expect(screen.getByRole('navigation', { name: 'Battery sections' })).toBeInTheDocument();
});

it('keeps software fallback scoped to the vehicle and tire warnings independent of completeness', () => {
  fixture.health.data = { vehicle_id: 'other', current_software_version: 'wrong' };
  const reading = {
    tire_fl_psi: 10, tire_fr_psi: 42, tire_rl_psi: 42, tire_rr_psi: null,
    charger_state: 'disconnected', ota_status: 'installing_update',
  } as unknown as VehicleStatus;
  const view = render(<RVehicleDetails vehicle={vehicle} status={reading} />);
  expect(screen.getByText('Version unknown')).toBeInTheDocument();
  expect(screen.getByText('installing update')).toBeInTheDocument();
  expect(screen.getByText('Readings incomplete')).toBeInTheDocument();
  expect(screen.getByText('Check tire pressure')).toBeInTheDocument();
  expect(screen.getByText('Not charging')).toBeInTheDocument();
  fixture.health.data = { vehicle_id: vehicle.id, current_software_version: '1.2.3' };
  view.rerender(<RVehicleDetails vehicle={vehicle} status={{ ...reading, ota_status: 'idle', ota_available_version: '0.0.0' }} />);
  expect(screen.getByText('1.2.3')).toBeInTheDocument();
  expect(screen.getByText('No update reported')).toBeInTheDocument();
  fixture.health.isPlaceholderData = true;
  view.rerender(<RVehicleDetails vehicle={vehicle} status={reading} />);
  expect(screen.getByText('Version unknown')).toBeInTheDocument();
});

it('keeps lock glyphs aligned with the three accessible lock states', () => {
  const view = render(<RLockIcon locked={null} />);
  expect(view.container.querySelector('.lucide-circle-question-mark')).toBeInTheDocument();
  expect(view.container.querySelector('[data-locked]')).toBeNull();
  for (const locked of [true, false]) {
    view.rerender(<RLockIcon locked={locked} />);
    expect(view.container.querySelector('svg')).toHaveAttribute('data-locked', String(locked));
    expect(view.container.querySelector('.r-lock-shackle')).toHaveAttribute('d', 'M8 10V6a4 4 0 0 1 8 0v4');
    expect(view.container.querySelector('.lucide-circle-question-mark')).toBeNull();
  }
});

it('keeps optional health identity fields and artwork fallbacks exact', () => {
  const view = render(<RHealthHero vehicle={undefined} name="Test" model="" vin={null} source={null} fallback="/fallback.webp">Summary</RHealthHero>);
  expect(screen.getByText('Vehicle identity pending telemetry')).toBeInTheDocument();
  expect(view.container.querySelector('small')).toBeNull();
  expect(screen.getByAltText('Vehicle three-quarter view')).toHaveAttribute('data-fallback', '/fallback.webp');
  view.rerender(<RHealthHero vehicle={vehicle} name="Test" model="R1T" vin="" source="/art.webp" fallback={null}>Summary</RHealthHero>);
  expect(screen.getByText('R1T')).toBeInTheDocument();
  expect(view.container.querySelector('small')).toBeNull();
  expect(screen.getByText('Summary')).toBeInTheDocument();
});

it('admits only exact battery routes and the configured map provider', () => {
  const view = render(<RBatteryNavigation pathname="/battery/phantom-drain" />);
  expect(screen.getAllByRole('link').map(link => link.getAttribute('href'))).toEqual(['/battery', '/battery/phantom-drain']);
  for (const pathname of ['/battery/', '/battery/phantom-drain/', '/battery/unrelated', '/']) {
    view.rerender(<RBatteryNavigation pathname={pathname} />);
    expect(view.container).toBeEmptyDOMElement();
  }
  for (const provider of ['', 'none', 'mapbox', 'openfreemap']) {
    fixture.basemap = provider;
    view.rerender(<RMapPreferences preferences={undefined} />);
    expect(view.container.querySelector('[data-map-style]') !== null).toBe(provider === 'openfreemap');
  }
  expect(view.container.firstChild).toHaveAttribute('data-map-style', 'follow-theme');
});
