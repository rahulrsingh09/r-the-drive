// Synthetic data for the Insights unit tests; no live vehicle or account is used.
import type { ChargeSession, Place, Trip } from '@panda/types';

export const homePlace: Place = {
  id: 'place-home', name: 'Home', latitude: 47.71355, longitude: -122.20369, radius_m: 75, is_home: true, is_work: false, address: null,
  charging: { id: 'cp-home', name: 'Home charging (PSE Schedule 7)', billing_type: 'per_kwh', rate: 0.189119, session_fee: 0, currency: 'USD', timezone: 'America/Los_Angeles', tou_periods: [] },
};
export const officePlace: Place = {
  id: 'place-office', name: 'Office', latitude: 47.61099, longitude: -122.19998, radius_m: 120, is_home: false, is_work: true, address: null,
  charging: { id: 'cp-office', name: 'Office charging (free)', billing_type: 'free', rate: 0, session_fee: 0, currency: 'USD', timezone: 'America/Los_Angeles', tou_periods: [] },
};
export const places = [homePlace, officePlace];

const charge = (id: string, over: Partial<ChargeSession>): ChargeSession => ({
  id, vehicle_id: 'v1', started_at: '2026-10-07T04:00:00Z', ended_at: '2026-10-07T07:00:00Z', location_name: null, charger_type: 'ac',
  energy_added_kwh: 20, duration_min: 180, soc_start: 40, soc_end: 62, peak_power_kw: 11, cost_usd: null, ...over,
});

export const sessions: ChargeSession[] = [
  charge('c-home-1', { location_name: 'Home', location_lat: 47.71356, location_lng: -122.2037, cost_usd: 3.78, cost_method: 'profile' }),
  charge('c-office-1', { started_at: '2026-10-08T17:00:00Z', ended_at: '2026-10-08T21:00:00Z', location_name: 'Office', location_lat: 47.611, location_lng: -122.2, energy_added_kwh: 30, cost_usd: 0, cost_method: 'profile' }),
  charge('c-dc-1', { started_at: '2026-10-14T20:00:00Z', ended_at: '2026-10-14T20:40:00Z', location_name: 'Near Ridgefield', location_lat: 45.81, location_lng: -122.74, charger_type: 'dc', energy_added_kwh: 40, cost_usd: 21.25, cost_method: 'rivian_paid_total', soc_start: 22, soc_end: 70, peak_power_kw: 190 }),
  charge('c-mill-1', { started_at: '2026-10-15T18:00:00Z', ended_at: '2026-10-15T19:00:00Z', location_name: 'Mill Creek', location_lat: 47.828, location_lng: -122.212, energy_added_kwh: 4 }),
  charge('c-open', { started_at: '2026-10-16T18:00:00Z', ended_at: null, location_name: 'Home', location_lat: 47.71356, location_lng: -122.2037 }),
];

const trip = (id: string, over: Partial<Trip>): Trip => ({
  id, vehicle_id: 'v1', started_at: '2026-10-07T16:00:00Z', ended_at: '2026-10-07T16:30:00Z', distance_mi: 20, duration_min: 30,
  energy_used_kwh: 6, efficiency_wh_mi: 300, max_speed_mph: 60, drive_mode: null, soc_start: 70, soc_end: 63,
  start_place: 'Home', end_place: 'Office', start_odometer_mi: 1000, end_odometer_mi: 1020, outside_temp_c: 10, ...over,
});

export const trips: Trip[] = [
  trip('t1', {}),
  trip('t2', { started_at: '2026-10-08T01:00:00Z', ended_at: '2026-10-08T01:30:00Z', start_place: 'Office', end_place: 'Home', start_odometer_mi: 1021, end_odometer_mi: 1041, efficiency_wh_mi: 260, outside_temp_c: 15 }),
  trip('t3', { started_at: '2026-10-14T16:00:00Z', ended_at: '2026-10-14T19:00:00Z', distance_mi: 120, energy_used_kwh: 40, efficiency_wh_mi: 333, start_place: 'Home', end_place: 'Near Kelso', start_odometer_mi: 1041, end_odometer_mi: 1161, outside_temp_c: 6 }),
  trip('t4', { started_at: '2026-10-15T16:00:00Z', ended_at: '2026-10-15T16:05:00Z', distance_mi: 1.2, energy_used_kwh: null, efficiency_wh_mi: null, start_place: 'Home', end_place: 'Home', start_odometer_mi: 1161, end_odometer_mi: 1162.2 }),
];
