import { useEffect, useRef, useSyncExternalStore } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { api, queryKeys, themeClient, useAuth, useAuthReady } from '@panda/hooks';
import { resolveThemeRuntimeResponse } from '@panda/ui/lib/theme';
import { APP_TIMEZONE_CHANGE_EVENT, getAppTimezone, setAppTimezone } from '@panda/ui/lib/dateTime';

function subscribeTimezone(notify: () => void) {
  window.addEventListener(APP_TIMEZONE_CHANGE_EVENT, notify);
  return () => window.removeEventListener(APP_TIMEZONE_CHANGE_EVENT, notify);
}

export function useRootPreferences() {
  useSyncExternalStore(subscribeTimezone, getAppTimezone, getAppTimezone);
  const authReady = useAuthReady();
  const accessToken = useAuth(state => state.accessToken);
  const userId = useAuth(state => state.userId);
  const client = useQueryClient();
  const account = useRef(userId);
  const sessionReady = authReady && !!accessToken;
  const accountReady = sessionReady && !!userId;
  const theme = useQuery({
    queryKey: queryKeys.themePreferences.forUser(userId ?? 'signed-out'),
    queryFn: () => themeClient.getPreferences(), enabled: accountReady,
  });
  const timezone = useQuery({
    queryKey: queryKeys.appTimezone.current, queryFn: () => api.getAppTimezone(), enabled: sessionReady,
  });
  useEffect(() => {
    const changedAccount = account.current !== userId;
    account.current = userId;
    if (changedAccount) void client.invalidateQueries({ queryKey: queryKeys.unitPreferences.current });
    const nextTimezone = timezone.data?.timezone;
    if (nextTimezone) setAppTimezone(nextTimezone);
  }, [client, userId, timezone.data?.timezone]);
  const resolved = resolveThemeRuntimeResponse(theme.data);
  return {
    preferences: accountReady ? resolved.preferences ?? null : null,
    resolvedTheme: resolved.resolvedTheme,
  };
}
