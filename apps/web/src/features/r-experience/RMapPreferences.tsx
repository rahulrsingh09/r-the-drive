import React from 'react';
import { useBasemapConfig } from '@panda/hooks';
import type { UserPreferencesResponse } from '@panda/types';
import { AppearanceSettings } from '../settings/AppearanceSettings';

const providerSettings = new Map([['openfreemap', AppearanceSettings]]);

export function RMapPreferences({ preferences }: { preferences: UserPreferencesResponse | undefined }) {
  const basemap = useBasemapConfig();
  const Settings = providerSettings.get(basemap.data?.resolved_provider ?? '');
  if (!Settings) return null;
  return <Settings mapStyle={preferences?.map_style ?? 'follow-theme'} />;
}
