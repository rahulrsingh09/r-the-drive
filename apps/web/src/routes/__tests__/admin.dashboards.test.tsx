import React from 'react';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@panda/ui/primitives', async () => {
  const m = await import('../../test/mockPrimitives');
  return m;
});

const adminRouteMocks = vi.hoisted(() => ({
  navigate: vi.fn(),
  apiFetch: vi.fn(),
  invalidateQueries: vi.fn(),
}));

vi.mock('@tanstack/react-router', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@tanstack/react-router')>();
  return { ...actual, useNavigate: () => adminRouteMocks.navigate };
});

vi.mock('@panda/hooks', () => ({
  api: {
    apiFetch: adminRouteMocks.apiFetch,
  },
}));

vi.mock('@panda/dashboards', () => ({
  useDashboards: () => ({
    data: [
      {
        id: 'default-1',
        slug: 'dashboard',
        name: 'Overview',
        isDefault: true,
        isLocked: false,
        ownerId: null,
        widgets: [],
      },
    ],
    isLoading: false,
  }),
}));

vi.mock('../../components/layout/AppLayout', () => ({
  AppLayout: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));
vi.mock('../../components/layout/AuthGuard', () => ({
  AuthGuard: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));

import { adminDashboardsRoute } from '../admin.dashboards';

const AdminDashboardsPage = adminDashboardsRoute.options.component as React.ComponentType;

function renderPage() {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  });
  vi.spyOn(queryClient, 'invalidateQueries').mockImplementation(adminRouteMocks.invalidateQueries as typeof queryClient.invalidateQueries);

  return render(
    <QueryClientProvider client={queryClient}>
      <AdminDashboardsPage />
    </QueryClientProvider>,
  );
}

describe('Admin dashboards route', () => {
  beforeEach(() => {
    adminRouteMocks.navigate.mockReset();
    adminRouteMocks.apiFetch.mockReset();
    adminRouteMocks.invalidateQueries.mockReset();
    adminRouteMocks.apiFetch.mockResolvedValue({ ok: true });
  });

  it('opens the exact saved dashboard without any layout administration actions', async () => {
    const user = userEvent.setup();
    renderPage();
    expect(screen.getByText('Overview')).toBeInTheDocument();
    expect(screen.queryByTitle('Edit (admin)')).toBeNull();
    expect(screen.queryByTitle('Lock')).toBeNull();
    expect(screen.queryByTitle('Unlock')).toBeNull();
    await user.click(screen.getByTitle('View'));
    expect(adminRouteMocks.navigate).toHaveBeenCalledWith({
      to: '/d/$slug', params: { slug: 'dashboard' }, search: { dashboardId: 'default-1' },
    });
    expect(adminRouteMocks.apiFetch).not.toHaveBeenCalled();
    expect(adminRouteMocks.invalidateQueries).not.toHaveBeenCalled();
  });
});
