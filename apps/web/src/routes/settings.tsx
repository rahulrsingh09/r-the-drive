import { createRoute, useSearch } from '@tanstack/react-router';
import { z } from 'zod';
import { SettingsContent, rSettingsSection } from '../features/settings/SettingsPage';
import { ProtectedRoute } from '../components/layout/ProtectedRoute';
import { rootRoute } from './__root';

export const settingsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/settings',
  validateSearch: z.object({
    // Old section names (units, places, api, ...) still parse; they open the card that replaced them, or the top.
    section: z.string().optional().catch(undefined),
    oidc: z.enum(['linked']).optional().catch(undefined),
    password: z.enum(['set']).optional().catch(undefined),
    error: z.enum(['oidc_cancelled', 'oidc_failed', 'oidc_denied', 'oidc_expired']).optional().catch(undefined),
  }),
  component: SettingsPage,
});

function SettingsPage() {
  const search = useSearch({ from: '/settings' });
  const feedback = search.password
    ? { oidcFeedback: 'Recovery password set. Other sessions were signed out.', oidcFeedbackKind: 'success' as const }
    : search.oidc
      ? { oidcFeedback: 'SSO is now linked to this account.', oidcFeedbackKind: 'success' as const }
      : search.error
        ? { oidcFeedback: 'SSO did not finish. It was cancelled or failed.', oidcFeedbackKind: 'error' as const }
        : {};
  return (
    <ProtectedRoute>
      <SettingsContent initialSection={rSettingsSection(search.section)} {...feedback} />
    </ProtectedRoute>
  );
}

export { SettingsContent } from '../features/settings/SettingsPage';
