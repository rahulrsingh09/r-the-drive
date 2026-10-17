import React from 'react';
import type { Vehicle } from '@panda/types';
import { configuredVehicleIdentity } from './readings';
import heroUrl from './assets/r2-hero.webp';

export function RConfiguredVehicleArtwork({ vehicle, className, children }: {
  vehicle: Vehicle | undefined; className?: string | undefined; children: React.ReactNode;
}) {
  const identity = configuredVehicleIdentity(vehicle);
  if (identity === null) return <>{children}</>;
  const alt = `${identity.color} ${identity.model} ${identity.trim} with ${identity.wheels} wheels`;
  return <img src={heroUrl} alt={alt} className={className} loading="lazy" />;
}
