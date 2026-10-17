import React from 'react';
import { createRootRouteWithContext, Outlet } from '@tanstack/react-router';
import type { QueryClient } from '@tanstack/react-query';
import { ThemeRuntimeProvider } from '@panda/ui/lib/theme';
import { RExperienceRoot } from '../features/r-experience/RExperienceRoot';
import { useRootPreferences } from '../hooks/useRootPreferences';

interface RouterContext {
  queryClient: QueryClient;
}

export const rootRoute = createRootRouteWithContext<RouterContext>()({
  component: Root,
});

function Root() {
  const { preferences, resolvedTheme } = useRootPreferences();
  return (
    <ThemeRuntimeProvider preferences={preferences} resolvedTheme={resolvedTheme}>
      <RExperienceRoot><Outlet /></RExperienceRoot>
    </ThemeRuntimeProvider>
  );
}
