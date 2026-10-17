import { createRoute } from '@tanstack/react-router';
import { z } from 'zod';
import { rootRoute } from './__root';
import { LoginPage } from '../components/auth/LoginPage';

export const loginRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/login',
  validateSearch: z.object({
    redirect: z.string().optional(),
    password_changed: z.literal('1').optional(),
    error: z.enum(['oidc_cancelled', 'oidc_expired', 'oidc_denied', 'oidc_failed']).optional(),
  }),
  component: LoginPage,
});

export { LoginPage };
