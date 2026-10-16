import React from 'react';
import type { Vehicle } from '@panda/types';
import { isConfiguredR2 } from './readings';
import heroUrl from './assets/r2-hero.webp';

export function RConfiguredVehicleArtwork({ vehicle, className, children }: {
  vehicle: Vehicle | undefined; className?: string | undefined; children: React.ReactNode;
}) {
  return isConfiguredR2(vehicle)
    ? <img src={heroUrl} alt="Catalina Cove R2 Performance with 21-inch wheels" className={className} loading="lazy" />
    : <>{children}</>;
}
