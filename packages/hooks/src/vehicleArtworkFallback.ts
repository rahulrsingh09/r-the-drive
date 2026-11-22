import type { VehicleImage, VehicleImagePair, VehicleImages } from '@panda/types';

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

export function normalizeVehicleArtworkModel(
  model: string | null | undefined,
): VehicleArtworkFallbackModel | null {
  const normalized = (model ?? '').trim().toLowerCase().replace(/[^a-z0-9]/g, '');
  if (normalized.includes('r1t')) return 'r1t';
  if (normalized.includes('r1s')) return 'r1s';
  if (normalized.includes('r2s') || normalized.includes('r2')) return 'r2';
  return null;
}

export function getVehicleArtworkFallback(
  model: string | null | undefined,
  usage: VehicleArtworkUsage,
): string | null {
  const normalizedModel = normalizeVehicleArtworkModel(model);
  // The packaged "r2s" pictures show an R1S body. An R2 shows only the owner's own
  // images, or a neutral placeholder, never another model.
  if (!normalizedModel || normalizedModel === 'r2') return null;
  return VEHICLE_ARTWORK_FALLBACKS[normalizedModel][usage];
}

export interface ResolvedVehicleArtwork {
  light: string | null;
  dark: string | null;
  fallback: string | null;
  /**
   * The owner's own Rivian images in turning order, one set per design. Only
   * the `vehicle-card` usage fills this; the sets are empty without API images.
   */
  angles?: { light: VehicleAngleImage[]; dark: VehicleAngleImage[] };
}

/** A turn angle of the owner's Rivian artwork, in turning order. */
export type VehicleTurnAngle = 'front' | 'front-three-quarter' | 'side' | 'rear-three-quarter' | 'rear';

export interface VehicleAngleImage {
  angle: VehicleTurnAngle;
  /** Name of the angle for assistive technology, for example "Front three-quarter". */
  label: string;
  url: string;
}

const TURN_ORDER: readonly VehicleTurnAngle[] = ['front', 'front-three-quarter', 'side', 'rear-three-quarter', 'rear'];
const TURN_LABELS: Record<VehicleTurnAngle, string> = {
  front: 'Front',
  'front-three-quarter': 'Front three-quarter',
  side: 'Side',
  'rear-three-quarter': 'Rear three-quarter',
  rear: 'Rear',
};

function normalizedPlacement(image: VehicleImage) {
  return image.placement.trim().toLowerCase().replace(/[_\s]+/g, '-');
}

function imageSearchText(image: VehicleImage) {
  return `${image.placement} ${image.design ?? ''} ${image.url} ${JSON.stringify(image.metadata ?? {})}`.toLowerCase();
}

function findSemanticImage(
  images: VehicleImages | null | undefined,
  predicate: (image: VehicleImage, text: string) => boolean,
  design: 'light' | 'dark',
) {
  return images?.all?.find((image) => (
    predicate(image, imageSearchText(image))
    && (image.design?.toLowerCase().includes(design) || image.design == null)
  ))?.url ?? null;
}

function findChargingImage(images: VehicleImages | null | undefined, design: 'light' | 'dark') {
  return findSemanticImage(images, (image, text) => {
    const placement = normalizedPlacement(image);
    return (placement.includes('side') && placement.includes('charg'))
      || text.includes('side-charging')
      || text.includes('side_charging')
      || (placement === 'side' && (text.includes('charge port') || text.includes('port open')));
  }, design);
}

function findHealthImage(
  images: VehicleImages | null | undefined,
  kind: 'hero' | 'three-quarter' | 'plain-side' | 'tagged-fallback' | 'front',
  design: 'light' | 'dark',
) {
  return findSemanticImage(images, (image, text) => {
    const placement = normalizedPlacement(image);
    if (kind === 'hero') return text.includes('health-hero') && !text.includes('health-hero-fallback');
    if (kind === 'three-quarter') {
      return placement.includes('three-quarter')
        || text.includes('three-quarter')
        || text.includes('three_quarter')
        || text.includes('3-quarter');
    }
    if (kind === 'plain-side') return placement === 'side' && !text.includes('charg');
    if (kind === 'tagged-fallback') return text.includes('health-hero-fallback');
    return placement === 'front';
  }, design);
}

function findPlacement(images: VehicleImages | null | undefined, placements: string[], design: 'light' | 'dark') {
  for (const placement of placements) {
    const match = images?.all?.find((image) => (
      normalizedPlacement(image) === placement
      && (image.design?.toLowerCase() === design || image.design == null)
    ));
    if (match) return match.url;
  }
  return null;
}

/** The turn angle of one Rivian placement. Overhead, charging and unknown placements are not turn angles. */
function turnAngle(image: VehicleImage): VehicleTurnAngle | null {
  const placement = normalizedPlacement(image).replace(/-/g, '');
  if (/charg|overhead|top|bird|interior|cabin/.test(placement)) return null;
  const rear = /rear|back/.test(placement);
  if (placement.includes('quarter')) return rear ? 'rear-three-quarter' : 'front-three-quarter';
  if (placement.includes('side') || placement.includes('profile')) return 'side';
  if (placement.includes('front')) return 'front';
  return rear ? 'rear' : null;
}

function imageDesign(image: VehicleImage): 'light' | 'dark' | null {
  const design = image.design?.toLowerCase() ?? '';
  return design.includes('dark') ? 'dark' : design.includes('light') ? 'light' : null;
}

/** Rivian sends a photo and a cel art set ("vehicle:photo", "order:cel"). Keep one style in one turn. */
function imageStyle(image: VehicleImage) {
  const source = (image.metadata as { source?: unknown } | null | undefined)?.source;
  return typeof source === 'string' ? source.split(':').pop() ?? '' : '';
}

/**
 * The owner's Rivian images in turning order: front, front three-quarter, side,
 * rear three-quarter, rear. Overhead and charging images are excluded. Only
 * placements that exist are returned, at most one image per angle, all from one
 * design and one art style. The requested design wins; the other design is used
 * only when the requested design has no turn image. The first image per angle
 * wins, so the API order (large size first) decides between sizes.
 */
export function resolveVehicleAngleSet(
  images: VehicleImages | null | undefined,
  design: 'light' | 'dark',
): VehicleAngleImage[] {
  const groups = new Map<string, { design: 'light' | 'dark' | null; images: Map<VehicleTurnAngle, string> }>();
  for (const image of images?.all ?? []) {
    const angle = turnAngle(image);
    if (!angle || !image.url) continue;
    const imageDesignValue = imageDesign(image);
    const key = `${imageStyle(image)}|${imageDesignValue ?? ''}`;
    const group = groups.get(key) ?? { design: imageDesignValue, images: new Map() };
    groups.set(key, group);
    if (!group.images.has(angle)) group.images.set(angle, image.url);
  }
  if (groups.size === 0) {
    // Older responses carry only the typed pairs.
    const pairs: Array<[VehicleTurnAngle, VehicleImagePair | null | undefined]> = [
      ['front', images?.front], ['side', images?.side], ['rear', images?.rear],
    ];
    const pick = (wanted: 'light' | 'dark') => new Map(pairs.flatMap(([angle, pair]) => {
      const url = pair?.[wanted];
      return url ? [[angle, url] as [VehicleTurnAngle, string]] : [];
    }));
    groups.set('typed|light', { design: 'light', images: pick('light') });
    groups.set('typed|dark', { design: 'dark', images: pick('dark') });
  }
  const rank = (group: { design: 'light' | 'dark' | null }) => (group.design === design ? 2 : group.design === null ? 1 : 0);
  let best: { design: 'light' | 'dark' | null; images: Map<VehicleTurnAngle, string> } | null = null;
  for (const group of groups.values()) {
    if (group.images.size === 0) continue;
    if (!best
      || rank(group) > rank(best)
      || (rank(group) === rank(best) && group.images.size > best.images.size)) best = group;
  }
  if (!best) return [];
  const chosen = best.images;
  return TURN_ORDER.flatMap((angle) => {
    const url = chosen.get(angle);
    return url ? [{ angle, label: TURN_LABELS[angle], url }] : [];
  });
}

export function resolveVehicleArtwork(
  images: VehicleImages | null | undefined,
  model: string | null | undefined,
  usage: VehicleArtworkUsage,
): ResolvedVehicleArtwork {
  const typedPair = usage === 'overview'
    ? images?.overhead
    : usage === 'vehicle-card'
      ? images?.side
      : null;
  const placements = usage === 'overview'
    ? ['overhead']
    : usage === 'charging'
      ? ['side-charging', 'charging-side']
      : usage === 'health'
        ? ['health-hero', 'three-quarter']
        : ['side'];
  const healthLight = usage === 'health'
    ? findHealthImage(images, 'hero', 'light')
      ?? findHealthImage(images, 'hero', 'dark')
      ?? findHealthImage(images, 'three-quarter', 'light')
      ?? findHealthImage(images, 'three-quarter', 'dark')
      ?? images?.side?.light
      ?? images?.side?.dark
      ?? findHealthImage(images, 'plain-side', 'light')
      ?? findHealthImage(images, 'plain-side', 'dark')
      ?? findHealthImage(images, 'tagged-fallback', 'light')
      ?? findHealthImage(images, 'tagged-fallback', 'dark')
      ?? findHealthImage(images, 'front', 'light')
      ?? findHealthImage(images, 'front', 'dark')
    : null;
  const healthDark = usage === 'health'
    ? findHealthImage(images, 'hero', 'dark')
      ?? findHealthImage(images, 'hero', 'light')
      ?? findHealthImage(images, 'three-quarter', 'dark')
      ?? findHealthImage(images, 'three-quarter', 'light')
      ?? images?.side?.dark
      ?? images?.side?.light
      ?? findHealthImage(images, 'plain-side', 'dark')
      ?? findHealthImage(images, 'plain-side', 'light')
      ?? findHealthImage(images, 'tagged-fallback', 'dark')
      ?? findHealthImage(images, 'tagged-fallback', 'light')
      ?? findHealthImage(images, 'front', 'dark')
      ?? findHealthImage(images, 'front', 'light')
    : null;
  const chargingLight = usage === 'charging' ? findChargingImage(images, 'light') ?? findChargingImage(images, 'dark') : null;
  const chargingDark = usage === 'charging' ? findChargingImage(images, 'dark') ?? findChargingImage(images, 'light') : null;
  const light = healthLight
    ?? chargingLight
    ?? typedPair?.light
    ?? typedPair?.dark
    ?? findPlacement(images, placements, 'light')
    ?? findPlacement(images, placements, 'dark');
  const dark = healthDark
    ?? chargingDark
    ?? typedPair?.dark
    ?? typedPair?.light
    ?? findPlacement(images, placements, 'dark')
    ?? light;
  return {
    light: light ?? null,
    dark: dark ?? null,
    fallback: getVehicleArtworkFallback(model, usage),
    angles: usage === 'vehicle-card'
      ? { light: resolveVehicleAngleSet(images, 'light'), dark: resolveVehicleAngleSet(images, 'dark') }
      : { light: [], dark: [] },
  };
}
