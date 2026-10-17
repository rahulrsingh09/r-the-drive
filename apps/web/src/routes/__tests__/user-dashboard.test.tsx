import React from 'react';
import { render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const routeState = vi.hoisted(() => ({
  slug: 'custom-dashboard',
  search: {} as { edit?: 1; dashboardId?: string },
  navigate: vi.fn(),
}));

const shellState = vi.hoisted(() => ({
  props: null as null | {
    slug: string;
    dashboardId?: string;
    isEditMode?: boolean;
    onEditModeChange?: (next: boolean) => void;
  },
}));

vi.mock('@tanstack/react-router', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@tanstack/react-router')>();
  return {
    ...actual,
    useParams: () => ({ slug: routeState.slug }),
    useSearch: () => routeState.search,
    useNavigate: () => routeState.navigate,
  };
});

vi.mock('@panda/hooks', () => ({
  useMe: () => ({ data: { role: 'user' } }),
}));

vi.mock('@panda/dashboards', () => ({
  findOwnedDashboardBySlug: (dashboards: Array<{ slug: string; ownerId: string | null }> | undefined, slug: string) =>
    dashboards?.find((dashboard) => dashboard.slug === slug && dashboard.ownerId != null),
  isSystemDefaultDashboard: (config: { isDefault: boolean; ownerId: string | null }) =>
    config.isDefault && !config.ownerId,
  materializeSystemDashboardDraft: (draft: object, saved: object) => ({ ...draft, ...saved }),
  materializeUserDashboardDraft: (draft: object, owned?: object | null) => ({
    ...draft,
    ...(owned ?? {}),
    isDefault: false,
    isLocked: false,
  }),
  useCreateDashboard: () => ({ mutateAsync: vi.fn(), isPending: false }),
  useUpdateDashboard: () => ({ mutateAsync: vi.fn(), isPending: false }),
  useUpdateAdminDashboard: () => ({ mutateAsync: vi.fn(), isPending: false }),
  useCloneDashboard: () => ({ mutateAsync: vi.fn(), isPending: false }),
  downloadDashboardYaml: vi.fn(),
  importDashboardYaml: vi.fn(),
}));

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

vi.mock('../../components/layout/ProtectedRoute', () => ({
  ProtectedRoute: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));

vi.mock('../../components/dashboard/DashboardPageShell', () => ({
  DashboardPageShell: (props: {
    slug: string;
    dashboardId?: string;
    isEditMode?: boolean;
    onEditModeChange?: (next: boolean) => void;
  }) => {
    shellState.props = props;
    return (
      <div
        data-testid="dashboard-shell"
        data-slug={props.slug}
        data-edit-mode={props.isEditMode ? 'true' : 'false'}
      >
      </div>
    );
  },
}));

import { userDashboardRoute } from '../d.$slug';

const RouteComponent = userDashboardRoute.options.component as React.ComponentType;

describe('user dashboard route', () => {
  beforeEach(() => {
    routeState.slug = 'custom-dashboard';
    routeState.search = {};
    routeState.navigate.mockReset();
    shellState.props = null;
  });

  it('ignores manual edit input while preserving slug and dashboard identity', () => {
    routeState.search = { edit: 1, dashboardId: '11111111-1111-4111-8111-111111111111' };
    render(<RouteComponent />);
    expect(screen.getByTestId('dashboard-shell')).toHaveAttribute('data-slug', 'custom-dashboard');
    expect(screen.getByTestId('dashboard-shell')).toHaveAttribute('data-edit-mode', 'false');
    expect(shellState.props?.dashboardId).toBe(routeState.search.dashboardId);
    expect(shellState.props?.onEditModeChange).toBeUndefined();
    expect(routeState.navigate).not.toHaveBeenCalled();
  });

  it.each([1, '1', true, 'anything'])('normalizes legacy edit=%s search to read-only', (edit) => {
    const schema = userDashboardRoute.options.validateSearch as { parse: (input: unknown) => { edit?: unknown; dashboardId?: string } };
    const dashboardId = '11111111-1111-4111-8111-111111111111';
    expect(schema.parse({ edit, dashboardId })).toEqual({ edit: undefined, dashboardId });
    expect(() => schema.parse({ edit, dashboardId: 'invalid' })).toThrow();
  });

  it('redirects the overview alias regardless of a manual edit query but keeps exact identity', () => {
    const beforeLoad = userDashboardRoute.options.beforeLoad as (input: unknown) => unknown;
    expect(() => beforeLoad({ params: { slug: 'dashboard' }, search: { edit: 1 } })).toThrow();
    expect(beforeLoad({ params: { slug: 'dashboard' }, search: { edit: 1, dashboardId: 'saved' } })).toBeUndefined();
    expect(beforeLoad({ params: { slug: 'custom-dashboard' }, search: { edit: 1 } })).toBeUndefined();
  });
});
