import React from 'react';
import { createRoute, redirect } from '@tanstack/react-router';
import { z } from 'zod';
import { rootRoute } from './__root';
import { ProtectedRoute } from '../components/layout/ProtectedRoute';
import { UserDashboardPage } from '../components/dashboard/UserDashboardPage';

const searchSchema = z.object({
  edit: z.literal(1).optional(),
  dashboardId: z.string().uuid().optional(),
});

export const userDashboardRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/d/$slug',
  validateSearch: searchSchema,
  beforeLoad: ({ params, search }) => {
    if (params.slug === 'dashboard' && !search.edit && !search.dashboardId) throw redirect({ to: '/', replace: true });
  },
  component: UserDashboardPageWrapper,
});

function UserDashboardPageWrapper() {
  return <ProtectedRoute><UserDashboardPage /></ProtectedRoute>;
}
