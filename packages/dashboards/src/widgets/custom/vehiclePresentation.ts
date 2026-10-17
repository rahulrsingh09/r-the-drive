import React from 'react';
import type { VehicleImages, VehicleStatus } from '@panda/types';
import { resolveVehicleArtwork, getVehicleArtworkFallback } from '@panda/hooks';
import { getTireHealthTone, formatTireLabel } from '@panda/ui/lib/vehicleTires';
import {
  getDoorOverlayUrls,
  getOpenDoorStates,
  findSideChargingImage,
  findBestChargingSideOverlay,
} from './imageUtils';

type Corner = 'fl' | 'fr' | 'rl' | 'rr';
const CLOSURE_APPEARANCE = {
  unknown: 'border-border bg-bg-elevated/60 text-fg-tertiary',
  closed: 'border-status-positive/60 bg-bg-elevated/90 text-status-positive',
  open: 'border-accent/60 bg-bg-elevated/90 text-accent',
};

export function closurePresentation(value: boolean | null | undefined) {
  const state = value === false ? 'open' : value == null ? 'unknown' : 'closed';
  return {
    open: state === 'open',
    label: state === 'unknown' ? 'unavailable' : state,
    className: CLOSURE_APPEARANCE[state],
  };
}

export function chargeGauge(value: number | null | undefined, limit?: number | null) {
  const level = Math.max(0, Math.min(100, value ?? 0));
  const bound = limit ? Math.max(0, Math.min(100, limit)) : null;
  const filled = Math.round(level / 5);
  return {
    level,
    limit: bound,
    segments: Array.from({ length: 20 }, (_, index) => ({ index, filled: index < filled })),
  };
}

export function overviewArtwork(
  images: VehicleImages | null | undefined,
  model: string | undefined,
  status: VehicleStatus | null | undefined,
  demo: boolean,
  targetPressure: number | null | undefined
) {
  const resolved = resolveVehicleArtwork(images, model, 'overview');
  const fallback = resolved.light ?? resolved.dark;
  const closures = getOpenDoorStates(status);
  const tires = {} as Record<Corner, { value: string; tone: ReturnType<typeof getTireHealthTone> }>;
  for (const corner of ['fl', 'fr', 'rl', 'rr'] as const) {
    const psi = status?.[`tire_${corner}_psi`],
      health = status?.[`tire_${corner}_status`];
    tires[corner] = {
      value: formatTireLabel(psi, health),
      tone: getTireHealthTone({ psi, status: health, targetPsi: targetPressure }),
    };
  }
  return {
    baseOverheadLight: resolved.light,
    baseOverheadDark: resolved.dark,
    apiOverheadFallback: fallback,
    localOverheadFallback: resolved.fallback,
    overheadArtworkAvailable: Boolean(fallback ?? resolved.fallback),
    isR2RealArtwork: ['R2', 'R2S'].includes(model ?? '') && !demo && Boolean(fallback),
    overlaysLight: getDoorOverlayUrls(images?.all, closures, 'light'),
    overlaysDark: getDoorOverlayUrls(images?.all, closures, 'dark'),
    tires,
  };
}
export function chargingArtwork(
  images: VehicleImages | null | undefined,
  model: string | undefined,
  demo: boolean
) {
  const fallback = getVehicleArtworkFallback(model, 'charging');
  const light =
    findSideChargingImage(images?.all, 'light') ??
    findBestChargingSideOverlay(images?.all, 'light');
  const dark =
    findSideChargingImage(images?.all, 'dark') ??
    findBestChargingSideOverlay(images?.all, 'dark') ??
    light;
  return { fallback, light: demo ? null : light, dark: demo ? null : dark };
}
export function useVehicleArtworkFrame(
  source: string | null | undefined,
  width: number,
  height: number
) {
  const [aspect, acceptSize] = React.useReducer(
    (current: number, size: { width: number; height: number }) =>
      size.width > 0 && size.height > 0 ? size.height / size.width : current,
    2.25
  );
  React.useEffect(() => {
    const image = new Image();
    image.onload = () => acceptSize({ width: image.naturalWidth, height: image.naturalHeight });
    if (source) image.src = source;
  }, [source]);
  const limits = [
    height > 0 ? Math.max(120, (height - 34) / 1.12) : 216,
    (width > 0 ? Math.max(260, (width - 34) / 1.04) : 520) / aspect,
  ];
  const frameHeight = Math.round(Math.min(...limits));
  return { frameHeight, frameWidth: Math.round(frameHeight * aspect) };
}
