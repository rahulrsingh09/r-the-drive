import * as React from 'react';
import {
  BUILT_IN_THEMES,
  resolveTheme,
  type ResolvedTheme,
  type ThemeColorPair,
  type ThemeDefinition,
} from '@panda/themes';
import {
  DEFAULT_THEME_PREFERENCES,
  type ThemeMode,
  type ThemePalette,
  type ThemePreferences,
  type ThemePreferencesV2,
  type ThemeRef,
  type ThemePreferencesResponse,
} from '@panda/types';
import { getBrandAsset, type BrandAssetKind } from './brandAssets';
import { observeMedia, readMedia } from './browserRuntime';

export type ThemeSelection = ThemePreferences | ThemePreferencesV2;
export type ResolvedCustomTheme = ResolvedTheme;
export function resolveThemeRuntimeResponse(response: ThemePreferencesResponse | null | undefined) {
  const preferences = response?.preferences ?? null;
  const selected = preferences?.selection;
  if (selected?.kind !== 'custom') return { preferences, resolvedTheme: null };
  const specification = Object.assign({ theme: selected.baseThemeId }, selected.definition);
  return { preferences, resolvedTheme: resolveTheme(specification as Parameters<typeof resolveTheme>[0]) };
}
export interface ThemeRuntimeSnapshot {
  selectedMode: ThemeMode;
  effectiveMode: Exclude<ThemeMode, 'system'>;
  themeRef: ThemeRef;
  revision: number;
  legacyPalette: ThemePalette;
  cssVariables: Readonly<Record<string, string>>;
  chartColors: Readonly<Record<string, string>>;
  chartColorPairs: Readonly<Record<string, ThemeColorPair>>;
  brandAssets: Readonly<Record<BrandAssetKind, string>>;
}

const MEDIA = '(prefers-color-scheme: dark)';
const CHART_KEYS = ['accent','success','warning','danger','muted','emerald','amber','sky','violet','rose','teal','indigo','yellow','orange'] as const;
const SERIES_KEYS = Array.from({ length: 16 }, (_, index) => `series-${String(index + 1).padStart(2, '0')}`);
const listeners = new Set<() => void>();
let stopWatching: (() => void) | undefined;
let active: { selection: ThemeSelection; custom?: ResolvedTheme | null | undefined } = { selection: DEFAULT_THEME_PREFERENCES };
let snapshot = makeSnapshot(DEFAULT_THEME_PREFERENCES);

function makeSnapshot(input: ThemeSelection, custom?: ResolvedTheme | null): ThemeRuntimeSnapshot {
  const selectedMode = input.mode === 'light' || input.mode === 'system' ? input.mode : 'dark';
  const effectiveMode = selectedMode === 'system' ? (readMedia(MEDIA)?.matches ? 'dark' : 'light') : selectedMode;
  const requested = 'selection' in input ? input.selection : { kind: 'builtin' as const, themeId: input.palette };
  const useCustom = requested?.kind === 'custom' && custom;
  const legacyPalette = useCustom ? custom.sourceTheme : requested?.kind === 'builtin' && requested.themeId === 'rad' ? 'rad' : 'classic';
  const definition: ThemeDefinition = useCustom ? custom : BUILT_IN_THEMES[legacyPalette];
  const themeRef: ThemeRef = useCustom ? requested : { kind: 'builtin', themeId: legacyPalette };
  const tokens = definition.tokens[effectiveMode];
  const fallback = definition.series['series-01']!;
  const cssVariables: Record<string, string> = {};
  const chartColors: Record<string, string> = {};
  const chartColorPairs: Record<string, ThemeColorPair> = { ...definition.series };
  for (const key of Object.keys(tokens) as Array<keyof typeof tokens>) cssVariables[`--rm-${key}`] = tokens[key];
  for (const key of CHART_KEYS) {
    const alias = definition.chartAliases[key];
    const color = typeof alias === 'string' ? tokens[alias as keyof typeof tokens] : alias?.[effectiveMode];
    cssVariables[`--rm-chart-${key}`] = chartColors[key] = color ?? fallback[effectiveMode];
    chartColorPairs[key] = alias && typeof alias !== 'string' ? alias : fallback;
  }
  for (const [index, key] of SERIES_KEYS.entries()) {
    const color = definition.series[key as keyof typeof definition.series]![effectiveMode];
    chartColors[key] = cssVariables[`--rm-${key}`] = color;
    cssVariables[`--rm-${key}-text`] = `color-mix(in srgb, var(--rm-${key}) 50%, var(--rm-text-primary) 50%)`;
    if (index < 6) cssVariables[`--rm-map-route-${index}`] = tokens[`map-route-${index}` as keyof typeof tokens] ?? color;
  }
  const brandAssets = {} as Record<BrandAssetKind, string>;
  for (const kind of ['wordmark', 'logo', 'icon', 'favicon'] as const) brandAssets[kind] = definition.brandAssets[kind][effectiveMode];
  return { selectedMode, effectiveMode, themeRef, revision: themeRef.kind === 'custom' ? themeRef.revision : 0, legacyPalette, cssVariables, chartColors, chartColorPairs, brandAssets };
}

function publish(next: ThemeRuntimeSnapshot) {
  snapshot = next;
  if (typeof document !== 'undefined') {
    const { documentElement: root } = document;
    root.classList.remove('light', 'dark');
    root.classList.add(next.effectiveMode);
    Object.assign(root.dataset, {
      rmPalette: next.legacyPalette, rmThemeId: next.themeRef.themeId,
      rmThemeKind: next.themeRef.kind, rmThemeRevision: String(next.revision),
    });
    root.style.colorScheme = next.effectiveMode;
    for (const property in next.cssVariables) root.style.setProperty(property, next.cssVariables[property]!);
    const favicon = document.querySelector<HTMLLinkElement>('link[data-rm-favicon]');
    if (favicon) favicon.href = next.brandAssets.favicon || getBrandAsset('favicon', { palette: next.legacyPalette, dark: next.effectiveMode === 'dark' });
  }
  for (const listener of listeners) listener();
}
function watchSystem() {
  const media = readMedia(MEDIA);
  if (!media) return;
  stopWatching?.();
  stopWatching = observeMedia(media, () => {
    if (snapshot.selectedMode === 'system') publish(makeSnapshot(active.selection, active.custom));
  });
}
export function getThemeRuntimeSnapshot() { return snapshot; }
export function subscribeThemeRuntime(listener: () => void) {
  listeners.add(listener);
  watchSystem();
  return () => listeners.delete(listener);
}
export function applyThemeRuntime(selection: ThemeSelection, custom?: ResolvedTheme | null) {
  active = { selection, custom };
  publish(makeSnapshot(selection, custom));
  watchSystem();
  return snapshot;
}
export function applyLegacyThemePreferences(preferences: ThemePreferences) {
  const selection = active.selection;
  const preserveCustom = 'selection' in selection && selection.selection?.kind === 'custom'
    && preferences.palette === active.custom?.sourceTheme;
  return preserveCustom
    ? applyThemeRuntime({ ...selection, mode: preferences.mode }, active.custom)
    : applyThemeRuntime(preferences);
}
export function resetThemeRuntime() { return applyThemeRuntime(DEFAULT_THEME_PREFERENCES); }
export function useThemeRuntime() { return React.useSyncExternalStore(subscribeThemeRuntime, getThemeRuntimeSnapshot, getThemeRuntimeSnapshot); }
export function useThemeRevision() { return useThemeRuntime().revision; }

export interface ThemeRuntimeProviderProps { preferences?: ThemeSelection | null; resolvedTheme?: ResolvedTheme | null; children: React.ReactNode; }
export function ThemeRuntimeProvider({ preferences, resolvedTheme, children }: ThemeRuntimeProviderProps) {
  const useBeforePaint = typeof window === 'undefined' ? React.useEffect : React.useLayoutEffect;
  useBeforePaint(() => { applyThemeRuntime(preferences ?? DEFAULT_THEME_PREFERENCES, preferences ? resolvedTheme : undefined); }, [preferences, resolvedTheme]);
  return React.createElement(React.Fragment, null, children);
}
