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
  const sources = [
    { mode: 'light', source: resolved.light, classes: `${className ?? ''} dark:hidden` },
    { mode: 'dark', source: resolved.dark ?? resolved.light, classes: `${className ?? ''} hidden dark:block` },
  ];
  const available = sources.some(({ source }) => Boolean(source)) || resolved.fallback;
  if (!available) return <>{fallback}</>;
  return (
    <>
      {sources.map(({ mode, source, classes }) => (
        <AuthenticatedVehicleArtwork key={mode} source={source} fallbackSource={resolved.fallback} fallbackProps={{ className: classes }} alt="" className={classes} loading="lazy" />
      ))}
    </>
  );
}
