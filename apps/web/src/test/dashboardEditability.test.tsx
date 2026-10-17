// The canonical dashboard controller is exercised independently of the R presentation.
vi.mock('../features/r-experience/RDashboardSurface', () => ({ RDashboardSurface: ({ children }: { children: React.ReactNode }) => children }));
import React from 'react';
import { render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { DashboardConfig } from '@panda/dashboards';

const dashboardMocks = vi.hoisted(() => ({
  navigate: vi.fn(),
  routeSlug: 'custom-dashboard',
  routeSearch: {} as { edit?: 1; dashboardId?: string },
  updateDashboard: vi.fn(),
  createDashboard: vi.fn(),
  updateAdminDashboard: vi.fn(),
  cloneDashboard: vi.fn(),
}));

vi.mock('@tanstack/react-router', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@tanstack/react-router')>();
  return {
    ...actual,
    useNavigate: () => dashboardMocks.navigate,
    useParams: () => ({ slug: dashboardMocks.routeSlug }),
    useSearch: () => dashboardMocks.routeSearch,
  };
});

vi.mock('@tanstack/react-query', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@tanstack/react-query')>();
  return {
    ...actual,
    useQueryClient: () => ({
      getQueryData: vi.fn(),
      refetchQueries: vi.fn(),
      invalidateQueries: vi.fn(),
    }),
  };
});

vi.mock('@panda/hooks', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@panda/hooks')>();
  const connectedStatus = {
    charger_state: 'Connected',
    charger_status: 'chrgr_sts_connected_no_chrg',
    power_state: 'standby',
  };
  const emptyMetricBatch = { values: [], series: [] };
  return {
    ...actual,
    useAuth: () => ({
      defaultVehicleId: 'vehicle-1',
      activeVehicleId: null,
      setActiveVehicleId: vi.fn(),
    }),
    useVehicles: () => ({
      data: [{ id: 'vehicle-1', display_name: 'Demo R1T', model: 'R1T' }],
    }),
    useResolvedVehicleSelection: () => ({
      authReady: true,
      effectiveVehicleId: 'vehicle-1',
      vehicleSelectionReady: true,
      vehicles: [{ id: 'vehicle-1', display_name: 'Demo R1T', model: 'R1T' }],
    }),
    useMe: () => ({ data: { role: 'user' } }),
    useMetricCatalog: () => ({ data: [] }),
    useMetricBatch: () => ({ data: emptyMetricBatch, isFetching: false }),
    useCurrentVehicleStatus: () => ({
      data: connectedStatus,
      isFetching: false,
    }),
    useBatteryHealth: () => ({ data: null, isFetching: false }),
    useChargingSummary: () => ({ data: null, isFetching: false }),
    useEfficiencySummary: () => ({ data: null, isFetching: false }),
  };
});

vi.mock('../../../../packages/dashboards/src/WidgetHost', () => ({
  WidgetHost: ({ instance }: { instance: { id: string; title?: string; definitionId: string } }) => (
    <div data-testid={`widget-host-${instance.id}`}>{instance.title ?? instance.definitionId}</div>
  ),
}));

vi.mock('@panda/dashboards', async () => {
  const actual = await vi.importActual<typeof import('@panda/dashboards')>('@panda/dashboards');
  const overview = actual.getDefaultBySlug('dashboard');
  const customDashboard: DashboardConfig = {
    ...overview!,
    id: 'custom-dashboard-id',
    slug: 'custom-dashboard',
    name: 'Custom Dashboard',
    isDefault: false,
    isLocked: false,
    ownerId: 'user-1',
    widgets: [
      {
        id: 'custom-widget',
        componentType: 'sensor',
        definitionId: 'total_miles',
        title: 'Custom miles',
        layout: { x: 0, y: 0, w: 3, h: 2 },
        options: {},
      },
    ],
  };

  function configForSlug(slug: string) {
    const config = slug === 'custom-dashboard' ? customDashboard : actual.getDefaultBySlug(slug);
    return config;
  }

  return {
    ...actual,
    getDefaultBySlug: configForSlug,
    useDashboardBySlug: (slug: string) => ({
      data: configForSlug(slug),
      isLoading: false,
    }),
    useDashboardById: (id: string | null) => ({
      data: id ? customDashboard : undefined,
      isLoading: false,
    }),
    useUpdateDashboard: () => ({ mutateAsync: dashboardMocks.updateDashboard, isPending: false }),
    useCreateDashboard: () => ({ mutateAsync: dashboardMocks.createDashboard, isPending: false }),
    useUpdateAdminDashboard: () => ({ mutateAsync: dashboardMocks.updateAdminDashboard, isPending: false }),
    useCloneDashboard: () => ({ mutateAsync: dashboardMocks.cloneDashboard, isPending: false }),
  };
});

vi.mock('../components/layout/AppLayout', () => ({
  AppLayout: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));

vi.mock('../components/layout/ProtectedRoute', () => ({
  ProtectedRoute: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));

vi.mock('../components/layout/NoVehicleState', () => ({
  NoVehicleState: () => null,
}));

vi.mock('@panda/ui/primitives', () => ({
  PageLayout: ({
    title,
    titleAction,
    actions,
    children,
  }: {
    title: string;
    titleAction?: React.ReactNode;
    actions?: React.ReactNode;
    children: React.ReactNode;
  }) => (
    <div>
      <h1>{title}</h1>
      {titleAction}
      {actions}
      {children}
    </div>
  ),
  DateRangePicker: () => <div data-testid="date-range-picker" />,
  EfficiencyDisplayToggle: () => <button type="button">Toggle efficiency units</button>,
  SelectPicker: ({ value, options, onChange, ...props }: {
    value: string;
    options: Array<{ value: string; label: React.ReactNode }>;
    onChange: (value: string) => void;
  }) => (
    <select value={value} onChange={(event) => onChange(event.target.value)} {...props}>
      {options.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}
    </select>
  ),
  Tooltip: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));

import { DashboardPageShell } from '../components/dashboard/DashboardPageShell';
import { userDashboardRoute } from '../routes/d.$slug';

const RouteComponent = userDashboardRoute.options.component as React.ComponentType;

describe('dashboard editability', () => {
  beforeEach(() => {
    document.body.innerHTML = '';
    document.body.className = '';
    dashboardMocks.navigate.mockReset();
    dashboardMocks.updateDashboard.mockResolvedValue({});
    dashboardMocks.createDashboard.mockResolvedValue({});
    dashboardMocks.updateAdminDashboard.mockResolvedValue({});
    dashboardMocks.cloneDashboard.mockResolvedValue({ slug: 'custom-dashboard' });
    dashboardMocks.routeSlug = 'custom-dashboard';
    dashboardMocks.routeSearch = {};
    localStorage.setItem('rm-show-dashboard-edit-button:anonymous', 'true');
  });

  it.each(['dashboard', 'battery', 'charging', 'efficiency', 'trips'])('keeps %s in view mode even when edit is requested', (slug) => {
    const onEditModeChange = vi.fn();
    const state = vi.fn();
    render(<DashboardPageShell navKey={slug} slug={slug} title={slug} isEditMode enableDashboardEditing onEditModeChange={onEditModeChange} renderActions={state} />);
    expect(screen.queryByRole('button', { name: 'Edit dashboard' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Save dashboard changes' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Discard dashboard changes' })).toBeNull();
    expect(document.querySelector('[data-widget-frame="edit"]')).toBeNull();
    expect(document.querySelector('.react-resizable-handle')).toBeNull();
    const latest = state.mock.calls.at(-1)![0];
    expect(latest.isEditMode).toBe(false);
    expect(latest.activeConfig).toBe(latest.savedConfig);
    expect(latest).not.toHaveProperty('setLocalConfig');
    expect(latest).not.toHaveProperty('enterEdit');
    expect(latest.ctx).not.toHaveProperty('updateWidgetLayout');
    expect(onEditModeChange).not.toHaveBeenCalled();
    expect(dashboardMocks.updateDashboard).not.toHaveBeenCalled();
    expect(dashboardMocks.updateAdminDashboard).not.toHaveBeenCalled();
    expect(dashboardMocks.createDashboard).not.toHaveBeenCalled();
  });

  it('renders the saved custom dashboard unchanged for a manually entered edit query', () => {
    dashboardMocks.routeSearch = { edit: 1, dashboardId: 'custom-dashboard-id' };
    render(<RouteComponent />);
    expect(screen.getByTestId('widget-host-custom-widget')).toHaveTextContent('Custom miles');
    expect(document.querySelector('[data-widget-frame="edit"]')).toBeNull();
    expect(screen.queryByRole('button', { name: 'Duplicate' })).toBeNull();
    expect(screen.queryByTitle('Import from YAML')).toBeNull();
    expect(screen.getByTitle('Export as YAML')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Customize' })).toBeNull();
    expect(dashboardMocks.cloneDashboard).not.toHaveBeenCalled();
    expect(dashboardMocks.navigate).not.toHaveBeenCalled();
  });

  it('hides the page edit shortcut when the dashboard setting is off', () => {
    localStorage.removeItem('rm-show-dashboard-edit-button:anonymous');

    render(<RouteComponent />);

    expect(screen.queryByRole('button', { name: 'Edit dashboard' })).toBeNull();
  });
});
