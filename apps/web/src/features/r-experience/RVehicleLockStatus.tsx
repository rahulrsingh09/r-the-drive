import React from 'react';
import type { VehicleStatus } from '@panda/types';
import { RLockIcon } from './RLockIcon';
import { vehicleLockReading } from './vehicleLockReading';
import { lockPresentation } from './navigationModel';

export function RVehicleLockStatus({ status }: { status: VehicleStatus | null | undefined }) {
  const locked = vehicleLockReading(status);
  const { label, value } = lockPresentation(locked);
  return <span className="r-vehicle-lock" role="img" aria-label={label} title={label} data-locked={value}>
    <RLockIcon locked={locked} />
  </span>;
}
