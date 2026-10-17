import type { VehicleImages } from '@panda/types';

export type VehicleArtworkUsage = 'overview' | 'charging' | 'health' | 'vehicle-card';
export type VehicleArtworkFallbackModel = 'r1s' | 'r1t' | 'r2';

const VEHICLE_ARTWORK_FALLBACKS: Record<
  VehicleArtworkFallbackModel,
  Record<VehicleArtworkUsage, string>
> = {
  r1s: {
    overview: '/vehicle-images/fallbacks/r1s/overview.webp',
    charging: '/vehicle-images/fallbacks/r1s/charging.webp',
    health: '/vehicle-images/fallbacks/r1s/health.webp',
    'vehicle-card': '/vehicle-images/fallbacks/r1s/side.webp',
  },
  r1t: {
    overview: '/vehicle-images/fallbacks/r1t/overview.webp',
    charging: '/vehicle-images/fallbacks/r1t/charging.webp',
    health: '/vehicle-images/fallbacks/r1t/health.webp',
    'vehicle-card': '/vehicle-images/fallbacks/r1t/side.webp',
  },
  // Keep the packaged path stable for existing deployments and cached URLs.
  r2: {
    overview: '/vehicle-images/fallbacks/r2s/overview.webp',
    charging: '/vehicle-images/fallbacks/r2s/charging.webp',
    health: '/vehicle-images/fallbacks/r2s/health.webp',
    'vehicle-card': '/vehicle-images/fallbacks/r2s/side.webp',
  },
};

export function normalizeVehicleArtworkModel(model: string | null | undefined): VehicleArtworkFallbackModel | null {
  const compact = (model ?? '').toLowerCase().replace(/[^a-z0-9]/g, '');
  return (['r1t', 'r1s', 'r2'] as const).find((key) => compact.includes(key)) ?? null;
}

export function getVehicleArtworkFallback(model: string | null | undefined, usage: VehicleArtworkUsage): string | null {
  const family = normalizeVehicleArtworkModel(model);
  return family === null ? null : VEHICLE_ARTWORK_FALLBACKS[family][usage];
}

export interface ResolvedVehicleArtwork { light: string | null; dark: string | null; fallback: string | null; }
type Mode = 'light' | 'dark';
type Role = 'charging' | 'hero' | 'three-quarter' | 'plain-side' | 'tagged-fallback' | 'front';

function imageIndex(images: VehicleImages | null | undefined) {
  const semantic = new Map<string, string>();
  const placements = new Map<string, string>();
  for (const image of images?.all ?? []) {
    const placement = image.placement.trim().toLowerCase().replace(/[_\s]+/g, '-');
    const text = `${image.placement} ${image.design ?? ''} ${image.url} ${JSON.stringify(image.metadata ?? {})}`.toLowerCase();
    const design = image.design?.toLowerCase();
    const roles: Record<Role, boolean> = {
      charging: (placement.includes('side') && placement.includes('charg')) || text.includes('side-charging') || text.includes('side_charging') || (placement === 'side' && (text.includes('charge port') || text.includes('port open'))),
      hero: text.includes('health-hero') && !text.includes('health-hero-fallback'),
      'three-quarter': placement.includes('three-quarter') || text.includes('three-quarter') || text.includes('three_quarter') || text.includes('3-quarter'),
      'plain-side': placement === 'side' && !text.includes('charg'),
      'tagged-fallback': text.includes('health-hero-fallback'),
      front: placement === 'front',
    };
    for (const mode of ['light', 'dark'] as const) {
      const key = `${mode}:${placement}`;
      if ((design === mode || design == null) && !placements.has(key)) placements.set(key, image.url);
      if (design != null && !design.includes(mode)) continue;
      for (const [role, matches] of Object.entries(roles)) {
        const key = `${mode}:${role}`;
        if (matches && !semantic.has(key)) semantic.set(key, image.url);
      }
    }
  }
  return { semantic, placements };
}

export function resolveVehicleArtwork(images: VehicleImages | null | undefined, model: string | null | undefined, usage: VehicleArtworkUsage): ResolvedVehicleArtwork {
  const index = imageIndex(images);
  const placementOrder = { overview: ['overhead'], charging: ['side-charging', 'charging-side'], health: ['health-hero', 'three-quarter'], 'vehicle-card': ['side'] }[usage];
  const typed = usage === 'overview' ? images?.overhead : usage === 'vehicle-card' ? images?.side : null;
  const first = (values: Array<string | null | undefined>) => values.find((value) => value != null) ?? null;
  const preferred = (mode: Mode) => {
    const other = mode === 'light' ? 'dark' : 'light';
    const role = (name: Role) => [index.semantic.get(`${mode}:${name}`), index.semantic.get(`${other}:${name}`)];
    const ranked = usage === 'health'
      ? [...role('hero'), ...role('three-quarter'), images?.side?.[mode], images?.side?.[other], ...role('plain-side'), ...role('tagged-fallback'), ...role('front')]
      : usage === 'charging' ? role('charging') : [];
    return first([...ranked, typed?.[mode], typed?.[other]]);
  };
  const placement = (mode: Mode) => first(placementOrder.map((name) => index.placements.get(`${mode}:${name}`)));
  const light = preferred('light') ?? placement('light') ?? placement('dark');
  const dark = preferred('dark') ?? placement('dark') ?? light;
  return { light, dark, fallback: getVehicleArtworkFallback(model, usage) };
}
