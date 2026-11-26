import { describe, expect, it } from 'vitest';
import type { ChargeSession, Place, Trip } from '@panda/types';
import { toActivityCharge, toActivityTrip } from './activityModel';
import { haversineMiles, inferRoadTrips, resolveHomeAnchor } from './roadTripModel';

const home: Place = {
  id: 'home',
  name: 'Home',
  latitude: 47.6769,
  longitude: -122.206,
  radius_m: 150,
  is_home: true,
  is_work: false,
  address: null,
  charging: null,
};

const mkTrip = (
  id: string,
  start: string,
  end: string,
  startLat: number,
  startLng: number,
  endLat: number,
  endLng: number,
  miles: number,
  extra: Partial<Trip> = {},
) =>
  toActivityTrip(
    {
      id,
      vehicle_id: 'v1',
      started_at: start,
      ended_at: end,
      distance_mi: miles,
      duration_min: Math.max(10, Math.round(miles * 1.1)),
      energy_used_kwh: miles * 0.32,
      efficiency_wh_mi: 320,
      max_speed_mph: null,
      drive_mode: null,
      soc_start: 80,
      soc_end: 60,
      start_lat: startLat,
      start_lng: startLng,
      end_lat: endLat,
      end_lng: endLng,
      ...extra,
    },
    [home],
  )!;

const mkCharge = (id: string, start: string, end: string, lat: number, lng: number, extra: Partial<ChargeSession> = {}) =>
  toActivityCharge(
    {
      id,
      vehicle_id: 'v1',
      started_at: start,
      ended_at: end,
      location_name: null,
      location_lat: lat,
      location_lng: lng,
      charger_type: 'dc',
      energy_added_kwh: 45,
      duration_min: 30,
      soc_start: 20,
      soc_end: 75,
      peak_power_kw: 200,
      cost_usd: 21.5,
      ...extra,
    },
    [home],
  )!;

describe('roadTripModel inference & bundling', () => {
  it('never bundles daily commutes or local errands that stay near Home', () => {
    // 4 segments totaling 82 odometer miles, all within ~20 mi straight-line of Home.
    const trips = [
      mkTrip('d1', '2026-11-10T15:00:00Z', '2026-11-10T15:35:00Z', 47.6769, -122.206, 47.45, -122.21, 20),
      mkTrip('d2', '2026-11-10T18:00:00Z', '2026-11-10T18:35:00Z', 47.45, -122.21, 47.61, -122.03, 21),
      mkTrip('d3', '2026-11-10T21:00:00Z', '2026-11-10T21:35:00Z', 47.61, -122.03, 47.45, -122.21, 21),
      mkTrip('d4', '2026-11-11T00:00:00Z', '2026-11-11T00:35:00Z', 47.45, -122.21, 47.6769, -122.206, 20),
    ];
    expect(inferRoadTrips(trips, [], [home])).toHaveLength(0);
  });

  it('never bundles stationary multi-day airport parking close to Home', () => {
    const seaTacLat = 47.4502;
    const seaTacLng = -122.3088;
    expect(haversineMiles(home.latitude, home.longitude, seaTacLat, seaTacLng)).toBeLessThan(25);
    const trips = [
      mkTrip('a1', '2026-11-01T14:00:00Z', '2026-11-01T14:35:00Z', 47.6769, -122.206, seaTacLat, seaTacLng, 18),
      mkTrip('a2', '2026-11-05T20:00:00Z', '2026-11-05T20:35:00Z', seaTacLat, seaTacLng, 47.6769, -122.206, 18),
    ];
    expect(inferRoadTrips(trips, [], [home])).toHaveLength(0);
  });

  it('bundles a weekend away excursion and reflects individual charge cost totals', () => {
    // Leavenworth (~77 mi straight-line from Kirkland) across Fri -> Sun with DC charge.
    const trips = [
      mkTrip('w1', '2026-11-13T23:00:00Z', '2026-11-14T01:30:00Z', 47.6769, -122.206, 47.5962, -120.6615, 118, { end_place: 'Leavenworth' }),
      mkTrip('w2', '2026-11-15T20:00:00Z', '2026-11-15T22:30:00Z', 47.5962, -120.6615, 47.6769, -122.206, 120, { start_place: 'Leavenworth' }),
    ];
    const charges = [
      mkCharge('c1', '2026-11-14T20:00:00Z', '2026-11-14T20:30:00Z', 47.5962, -120.6615, { cost_usd: 22.5, energy_added_kwh: 50 }),
    ];
    const bundled = inferRoadTrips(trips, charges, [home]);
    expect(bundled).toHaveLength(1);
    expect(bundled[0]).toMatchObject({
      miles: 238,
      kwh: 50,
      cost: 22.5,
      unpricedCount: 0,
    });
    expect(bundled[0]!.days).toHaveLength(3);

    // Editing override respects Rename and Split.
    const renamed = inferRoadTrips(trips, charges, [home], { names: { [bundled[0]!.id]: 'Alpine Getaway' }, splitIds: [] });
    expect(renamed[0]!.name).toBe('Alpine Getaway');
    const split = inferRoadTrips(trips, charges, [home], { names: {}, splitIds: [bundled[0]!.id] });
    expect(split).toHaveLength(0);
  });

  it('infers Home from repeated overnight endpoints when no saved Home place exists', () => {
    const trips = [
      mkTrip('n1', '2026-11-01T16:00:00Z', '2026-11-02T05:30:00Z', 47.61, -122.20, 47.6769, -122.206, 8),
      mkTrip('n2', '2026-11-02T16:00:00Z', '2026-11-03T06:00:00Z', 47.61, -122.20, 47.677, -122.2061, 8),
    ];
    const anchor = resolveHomeAnchor(trips, []);
    expect(anchor?.inferred).toBe(true);
    expect(anchor?.lat).toBeCloseTo(47.6769, 2);
  });
});
