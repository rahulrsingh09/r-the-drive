import { BUILT_IN_THEMES } from '@panda/themes';
import type { ThemePalette } from '@panda/types';

export type BrandAssetKind = 'wordmark' | 'logo' | 'icon' | 'favicon';

export interface BrandAssetOptions {
  palette?: ThemePalette;
  dark?: boolean;
}

/** Single source of truth for app brand assets; docs keep their static classic assets. */
export function getBrandAsset(kind: BrandAssetKind, options: BrandAssetOptions = {}): string {
  const assets = BUILT_IN_THEMES[options.palette === 'rad' ? 'rad' : 'classic'].brandAssets;
  return assets[kind][options.dark === false ? 'light' : 'dark'];
}
