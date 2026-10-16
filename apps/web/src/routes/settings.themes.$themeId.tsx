import { createRoute, redirect } from '@tanstack/react-router';
import { rootRoute } from './__root';

export const settingsThemeStudioRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/settings/themes/$themeId',
  beforeLoad: () => { throw redirect({ to: '/settings', replace: true }); },
});
