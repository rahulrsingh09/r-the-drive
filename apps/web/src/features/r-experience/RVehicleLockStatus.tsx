import React from 'react';
import type { VehicleStatus } from '@panda/types';
import { RLockIcon } from './RLockIcon';
import { vehicleLockReading } from './vehicleLockReading';

export function RVehicleLockStatus({ status }: { status: VehicleStatus | null | undefined }) {
  const locked = vehicleLockReading(status);
  const label = locked === null ? 'Vehicle lock status unavailable' : locked ? 'Vehicle locked' : 'Vehicle unlocked';
  return <span className="r-vehicle-lock" role="img" aria-label={label} title={label} data-locked={locked ?? 'unknown'}>
    <RLockIcon locked={locked} />
  </span>;
}
