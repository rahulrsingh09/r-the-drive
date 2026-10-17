import React from 'react';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { DashboardPageShellRenderState } from '../components/dashboard/dashboardShellTypes';
import type { DashboardConfig } from '@panda/dashboards';

const fixture = vi.hoisted(() => ({
  role: 'admin', pathname: '/', download: vi.fn(), mutate: vi.fn(), rendered: [] as unknown[],
}));
vi.mock('@tanstack/react-router', () => ({
  useLocation: () => ({ pathname: fixture.pathname, search: { edit: 1 } }),
  useNavigate: () => fixture.mutate,
  Link: ({ to, search, activeOptions: _active, children, ...props }: React.PropsWithChildren<Record<string, unknown>>) =>
    <a {...props} href={`${to}${search ? `?${new URLSearchParams(search as Record<string, string>)}` : ''}`}>{children}</a>,
}));
vi.mock('@panda/hooks', () => ({ useMe: () => ({ data: { role: fixture.role } }) }));
vi.mock('@panda/dashboards', async importOriginal => ({
  ...await importOriginal<typeof import('@panda/dashboards')>(),
  useCloneDashboard: () => ({ mutateAsync: fixture.mutate }),
  useCreateDashboard: () => ({ mutateAsync: fixture.mutate }),
  downloadDashboardYaml: (config: unknown) => fixture.download(config),
  DashboardRenderer: ({ config }: { config: DashboardConfig }) => {
    fixture.rendered.push(config);
    return <div data-saved-widgets={JSON.stringify(config.widgets)} />;
  },
}));
vi.mock('@panda/ui/primitives', () => ({
  SelectPicker: ({ options, value, onChange, ...props }: {
    options: { value: string; label: string }[]; value: string; onChange: (value: string) => void;
  }) => <select {...props} value={value} onChange={event => onChange(event.target.value)}>
    {options.map(option => <option key={option.value} value={option.value}>{option.label}</option>)}
  </select>,
}));
vi.mock('../features/r-experience/RAppLayout', () => ({
  RAppLayout: ({ children }: React.PropsWithChildren) => <main>{children}</main>,
}));
vi.mock('../features/r-experience/RDashboardVehicle', () => ({
  HomeVehicleHeadingContext: React.createContext<string | undefined>(undefined),
}));
vi.mock('../features/r-experience/RRecentActivity', () => ({ RRecentActivity: () => <p>Recent activity</p> }));
import { getDefaultBySlug } from '@panda/dashboards';
import { RDashboardTools } from '../features/r-experience/RDashboardTools';
import { RDashboardSurface } from '../features/r-experience/RDashboardSurface';
import { RHomeWorkspace } from '../features/r-experience/RHomeWorkspace';
import { RSettingsNavigation } from '../features/r-experience/RSettingsNavigation';
import { RExplore } from '../features/r-experience/RExplore';
import { settingsDirectory } from '../features/r-experience/settingsModel';

beforeEach(() => { fixture.pathname = '/'; fixture.role = 'admin'; fixture.rendered.length = 0; });
afterEach(() => { cleanup(); vi.clearAllMocks(); });
const state = (config = getDefaultBySlug('dashboard')!) => ({
  activeConfig: config, savedConfig: config, timeframe: '7d', ctx: { dashboardSlug: config.slug },
}) as unknown as DashboardPageShellRenderState;
const Icon = () => <svg />;
const sections = ['dashboards', 'charts', 'units', 'jobs', 'other'].map(id => ({ id, label: id, icon: Icon }));

it('offers export without clone, import, file input or layout-library navigation', () => {
  const current = state();
  const view = render(<RDashboardTools state={current} />);
  expect(screen.queryByText('Duplicate dashboard')).toBeNull();
  expect(screen.queryByText('Import dashboard')).toBeNull();
  expect(screen.queryByText('Dashboard library')).toBeNull();
  expect(view.container.querySelector('input[type=file]')).toBeNull();
  fireEvent.click(screen.getByText('Export dashboard'));
  expect(fixture.download).toHaveBeenCalledWith(current.activeConfig);
  expect(fixture.mutate).not.toHaveBeenCalled();
  view.rerender(<RDashboardTools state={{ ...current, activeConfig: undefined }} />);
  fireEvent.click(screen.getByText('Export dashboard'));
  expect(fixture.download).toHaveBeenCalledTimes(1);
});

it('excludes restructuring from the settings directory, search and section picker', () => {
  const select = vi.fn();
  const view = render(<RSettingsNavigation sections={sections} active="directory" onSelect={select} />);
  expect(screen.queryByRole('button', { name: /dashboards/ })).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: /charts/ }));
  expect(select).toHaveBeenLastCalledWith('charts');
  fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'dashboards' } });
  expect(screen.getByRole('status')).toHaveTextContent('No settings match');
  view.rerender(<RSettingsNavigation sections={sections} active="charts" onSelect={select} />);
  expect(screen.queryByRole('option', { name: 'dashboards' })).toBeNull();
  expect(screen.getByRole('option', { name: 'jobs' })).toBeInTheDocument();
  fireEvent.change(screen.getByRole('combobox'), { target: { value: 'units' } });
  expect(select).toHaveBeenLastCalledWith('units');
  expect(settingsDirectory(sections, '').matches).toBe(4);
});

it('renders the settings directory for a stale dashboards selection without selecting it', () => {
  const select = vi.fn();
  render(<RSettingsNavigation sections={sections} active="dashboards" onSelect={select} />);
  expect(screen.getByRole('searchbox')).toBeInTheDocument();
  expect(screen.queryByRole('combobox')).toBeNull();
  expect(select).not.toHaveBeenCalled();
});

it.each(['admin', 'super_user', 'viewer', 'user', 'unknown'])('keeps Explore links and people permissions for %s without a layout library', role => {
  fixture.role = role;
  render(<RExplore />);
  expect(screen.queryByText('Dashboard library')).toBeNull();
  expect(screen.queryByRole('link', { name: /People & access/ }) !== null).toBe(['admin', 'super_user'].includes(role));
  expect(screen.getAllByRole('link').map(link => link.getAttribute('href'))).toEqual([
    '/efficiency', '/vehicle-health', '/battery', '/battery/phantom-drain', '/settings',
    ...(['admin', 'super_user'].includes(role) ? ['/users'] : []),
  ]);
});

it('ignores stale edit flags and edit URL hints while preserving home readiness', () => {
  // Deliberately inject legacy runtime input outside the shell's read-only type.
  const current = { ...state(), isEditMode: true } as unknown as DashboardPageShellRenderState;
  const view = render(<RDashboardSurface state={current} ready actions={null}>Route body</RDashboardSurface>);
  expect(screen.queryByText('Route body')).toBeNull();
  expect(view.container.querySelector('[data-r-default-layout]')).toHaveAttribute('data-r-default-layout', 'true');
  view.rerender(<RDashboardSurface state={current} ready={false} actions={null}>Route body</RDashboardSurface>);
  expect(screen.getByText('Route body')).toBeInTheDocument();
  fixture.pathname = '/battery';
  view.rerender(<RDashboardSurface state={current} ready actions={null}>Route body</RDashboardSurface>);
  expect(screen.getByRole('navigation', { name: 'Battery sections' })).toBeInTheDocument();
});

it('keeps saved custom widget identity, options and rebased home geometry without mutation', () => {
  const saved = structuredClone(getDefaultBySlug('dashboard')!);
  saved.id = 'saved-custom';
  saved.slug = 'saved-custom';
  saved.name = 'Synthetic saved layout';
  saved.widgets = [
    { ...saved.widgets.find(widget => widget.componentType === 'sensor')!, id: 'saved-sensor',
      layout: { x: 4, y: 11, w: 3, h: 2 }, options: { custom: 'retained' } },
    { ...saved.widgets.find(widget => widget.componentType === 'chart')!, id: 'saved-chart',
      layout: { x: 1, y: 17, w: 8, h: 5 }, title: 'Saved chart title', options: { custom: 42 } },
  ];
  const original = structuredClone(saved);
  const view = render(<RHomeWorkspace state={state(saved)} actions={<button>Timeframe</button>} activity={<p>History</p>} />);
  expect(saved).toEqual(original);
  expect(fixture.rendered).toEqual([
    { ...original, widgets: [{ ...original.widgets[0], layout: { x: 4, y: 0, w: 3, h: 2 } }] },
    { ...original, widgets: [{ ...original.widgets[1], layout: { x: 1, y: 0, w: 8, h: 5 } }] },
  ]);
  expect(view.container.querySelector('[data-r-default-layout]')).toBeNull();
  expect(screen.getByRole('button', { name: 'Timeframe' })).toBeInTheDocument();
  expect(fixture.mutate).not.toHaveBeenCalled();
});
