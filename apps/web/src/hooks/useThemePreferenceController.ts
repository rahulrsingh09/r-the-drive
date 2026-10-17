import * as React from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { queryKeys, themeClient, useAuth } from '@panda/hooks';
import type { ThemeMode, ThemePreferencesResponse } from '@panda/types';
import { applyThemePreferences, resolveThemeRuntimeResponse, useThemeRuntime } from '@panda/ui/lib/theme';
import { emitToast } from '../components/feedback/toast';

type ModeWrite = { mode: ThemeMode; owner: string };

export function useThemePreferenceController() {
  const client = useQueryClient();
  const userId = useAuth(state => state.userId);
  const runtime = useThemeRuntime();
  const inFlight = React.useRef<ModeWrite | null>(null);
  const preference = useQuery({
    queryKey: queryKeys.themePreferences.forUser(userId ?? 'signed-out'),
    queryFn: () => themeClient.getPreferences(), enabled: !!userId,
  });
  const mutation = useMutation({
    mutationFn: ({ mode, owner }: ModeWrite) => {
      const snapshot = client.getQueryData<ThemePreferencesResponse>(queryKeys.themePreferences.forUser(owner));
      if (!snapshot?.etag) return Promise.reject(new Error('Appearance preferences are still loading.'));
      if (owner !== useAuth.getState().userId) return Promise.reject(new Error('Account changed.'));
      const preferences = { ...snapshot.preferences, mode };
      return themeClient.updatePreferences(preferences, snapshot.etag);
    },
    onSettled: (response, error, request) => {
      if (inFlight.current === request) inFlight.current = null;
      if (request.owner !== useAuth.getState().userId) return;
      const preferenceKey = queryKeys.themePreferences.forUser(request.owner);
      if (error) {
        emitToast({ title: 'Appearance could not be saved', message: 'Your previous appearance is unchanged. Please try again.', variant: 'error' });
        void client.invalidateQueries({ queryKey: preferenceKey });
        return;
      }
      if (!response) return;
      client.setQueryData(preferenceKey, response);
      const resolved = resolveThemeRuntimeResponse(response);
      if (resolved.preferences) applyThemePreferences(resolved.preferences, resolved.resolvedTheme);
      void client.invalidateQueries({ queryKey: queryKeys.unitPreferences.current });
    },
  });
  const onModeChange = React.useCallback((mode: ThemeMode) => {
    const ready = userId && preference.data?.etag && inFlight.current?.owner !== userId;
    if (!ready) return;
    const request = { mode, owner: userId };
    inFlight.current = request;
    mutation.mutate(request);
  }, [mutation, preference.data?.etag, userId]);
  const isPending = !preference.data?.etag || (mutation.isPending && mutation.variables?.owner === userId);
  return { mode: preference.data?.preferences.mode ?? runtime.selectedMode, onModeChange, isPending, error: mutation.error };
}
