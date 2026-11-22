import type { Place, VehicleStatus } from '@panda/types';
import type { ChargeRow, DayKey, TripRow } from './insightsModel';

/** What every Insights tab reads. The page loads it once. */
export interface InsightsData {
  vehicleId: string;
  userId: string | null;
  timezone: string;
  today: DayKey;
  /** The first day with a trip or a charge. */
  first: DayKey;
  trips: TripRow[];
  charges: ChargeRow[];
  places: Place[];
  status: VehicleStatus | null;
}

/** The Activity list, filtered to the charges at one place. The activity node owns the query names. */
export const activityPlaceSearch = (place: string) => ({ filter: 'charges', place });
export const ACTIVITY_PATH = '/trips';
