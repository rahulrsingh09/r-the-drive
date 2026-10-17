import type { VehicleStatus } from '@panda/types';
import { admittedTelemetry } from './telemetryModel';

export function vehicleLockReading(status: VehicleStatus | null | undefined): boolean | null {
  const reading = admittedTelemetry(status, 'doors_locked');
  switch (reading) {
    case true: return true;
    case false: return false;
    default: return null;
  }
}
