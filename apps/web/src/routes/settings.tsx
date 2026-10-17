import { settingsEntryProps } from '../lib/settingsEntryProps';
import { createRoute, useSearch } from '@tanstack/react-router';
import { z } from 'zod';
import { SettingsContent } from '../features/settings/SettingsPage';
import { ProtectedRoute } from '../components/layout/ProtectedRoute';
import { rootRoute } from './__root';

export const settingsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/settings',
  validateSearch: z.object({
    section: z
      .enum([
        'vehicles',
        'dashboards',
        'charts',
        'units',
        'places',
        'charging',
        'external',
        'api',
        'jobs',
        'raw',
        'backup',
        'appearance',
        'account',
        'authentication',
      ])
      .optional(),
    oidc: z.enum(['linked']).optional(),
    password: z.enum(['set']).optional(),
    error: z.enum(['oidc_cancelled', 'oidc_failed', 'oidc_denied', 'oidc_expired']).optional(),
  }),
  component: SettingsPage,
});

function SettingsPage() {
  const search = useSearch({ from: '/settings' });
  return (
    <ProtectedRoute>
      <SettingsContent
        {...settingsEntryProps(search)}
      />
    </ProtectedRoute>
  );
}

export { SettingsContent } from '../features/settings/SettingsPage';
