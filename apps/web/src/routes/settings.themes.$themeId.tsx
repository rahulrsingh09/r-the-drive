import { createRoute, redirect } from '@tanstack/react-router';
import { rootRoute } from './__root';

/** R has no theme studio; the theme button switches light and dark. Keep old links working. */
export const settingsThemeStudioRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/settings/themes/$themeId',
  beforeLoad: () => { throw redirect({ to: '/settings', replace: true }); },
});
