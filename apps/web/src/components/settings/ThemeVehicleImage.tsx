import type React from 'react';
import { AuthenticatedVehicleArtwork, resolveVehicleArtwork } from '@panda/hooks';
import type { VehicleImages } from '@panda/types';

export function ThemeVehicleImage({
  images,
  model,
  placement,
  className,
  fallback,
}: {
  images?: VehicleImages | null | undefined;
  model?: string | null | undefined;
  placement: 'side' | 'overhead' | 'front' | 'rear';
  className?: string;
  fallback: React.ReactNode;
}) {
  const resolved = resolveVehicleArtwork(images, model, placement === 'side' ? 'vehicle-card' : 'health');
  const light = resolved.light;
  const dark = resolved.dark ?? light;

  if (!light && !dark && !resolved.fallback) return <>{fallback}</>;

  return (
    <>
      <AuthenticatedVehicleArtwork source={light} fallbackSource={resolved.fallback} fallbackProps={{ className: `${className ?? ''} dark:hidden` }} alt="" className={`${className ?? ''} dark:hidden`} loading="lazy" />
      <AuthenticatedVehicleArtwork source={dark} fallbackSource={resolved.fallback} fallbackProps={{ className: `${className ?? ''} hidden dark:block` }} alt="" className={`${className ?? ''} hidden dark:block`} loading="lazy" />
    </>
  );
}
