import React from 'react';
import { createRoute } from '@tanstack/react-router';
import { rootRoute } from './__root';
import { ProtectedRoute } from '../components/layout/ProtectedRoute';
import { AdminDashboardsPage } from '../components/dashboard/AdminDashboardsPage';

export const adminDashboardsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/admin/dashboards',
  component: AdminDashboardsWrapper,
});

function AdminDashboardsWrapper() {
  return <ProtectedRoute><AdminDashboardsPage /></ProtectedRoute>;
}
