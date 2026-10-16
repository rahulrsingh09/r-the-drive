import React from 'react';
import { useBasemapConfig } from '@panda/hooks';
import type { UserPreferencesResponse } from '@panda/types';
import { AppearanceSettings } from '../settings/AppearanceSettings';

export function RMapPreferences({ preferences }: { preferences: UserPreferencesResponse | undefined }) {
  const basemap = useBasemapConfig();
  return basemap.data?.resolved_provider === 'openfreemap'
    ? <AppearanceSettings mapStyle={preferences?.map_style ?? 'follow-theme'} />
    : null;
}
