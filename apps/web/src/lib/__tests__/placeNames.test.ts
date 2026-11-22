import { describe, expect, it } from 'vitest';
import type { ChargeSession, Place, Trip } from '@panda/types';
import { chargePlace, nearestTown, shortPlace, tripPlace, tripTitleOf } from '../placeNames';

const home = { id: 'home', name: 'Home', latitude: 47.7136, longitude: -122.2037, radius_m: 150, is_home: true, is_work: false, address: null, charging: null } as Place;

const session = (over: Partial<ChargeSession>) => ({
  location_name: null, location_lat: null, location_lng: null, rivian_city: null, ...over,
}) as ChargeSession;

const trip = (over: Partial<Trip>) => ({
  start_lat: null, start_lng: null, end_lat: null, end_lng: null, start_place: null, end_place: null, start_address: null, end_address: null, ...over,
}) as Trip;

describe('nearestTown', () => {
  it('names the towns of the real drives and charges', () => {
    expect(nearestTown(45.8178, -122.6827)).toBe('Ridgefield');
    expect(nearestTown(46.1433, -122.8986)).toBe('Kelso');
    expect(nearestTown(47.8284, -122.2115)).toBe('Mill Creek');
    expect(nearestTown(45.5152, -122.6784)).toBe('Portland');
    expect(nearestTown(46.6621, -122.964)).toBe('Chehalis');
    expect(nearestTown(47.7136, -122.2037)).toBe('Kirkland');
    expect(nearestTown(47.611, -122.2)).toBe('Bellevue');
    expect(nearestTown(47.6062, -122.3321)).toBe('Seattle');
  });

  it('gives no town far from every known town or without a position', () => {
    expect(nearestTown(40.7, -74)).toBeNull();
    expect(nearestTown(null, -122)).toBeNull();
    expect(nearestTown(0, 0)).toBeNull();
  });
});

describe('chargePlace', () => {
  it('uses the saved place, then the nearest town, then coordinates', () => {
    expect(chargePlace(session({ location_name: '47.7136, -122.2037' }), home)).toEqual({ name: 'Home', approximate: false });
    expect(chargePlace(session({ location_name: '45.8178, -122.6827', location_lat: 45.8178, location_lng: -122.6827 }), null))
      .toEqual({ name: 'Near Ridgefield', approximate: true });
    expect(chargePlace(session({ location_name: '40.7000, -74.0000', location_lat: 40.7, location_lng: -74 }), null))
      .toEqual({ name: '40.7000, -74.0000', approximate: false });
    expect(chargePlace(session({}), null)).toEqual({ name: 'Unknown place', approximate: false });
  });

  it('keeps a short reported name', () => {
    expect(chargePlace(session({ location_name: 'Electrify America', location_lat: 45.8178, location_lng: -122.6827 }), null).name).toBe('Electrify America');
  });
});

describe('tripPlace', () => {
  it('names a trip end without a place name by the nearest town', () => {
    const t = trip({ start_lat: 47.7136, start_lng: -122.2037, start_place: 'Home', end_lat: 45.8178, end_lng: -122.6827 });
    const from = tripPlace(t, 'start', [home]);
    const to = tripPlace(t, 'end', [home]);
    expect(to).toEqual({ name: 'Near Ridgefield', approximate: true });
    expect(tripTitleOf(from, to)).toBe('Home → Ridgefield');
  });

  it('does not use a coordinate label as a place name', () => {
    expect(tripPlace(trip({ end_place: '46.1433, -122.8986', end_lat: 46.1433, end_lng: -122.8986 }), 'end', []).name).toBe('Near Kelso');
  });

  it('drops the "Near" mark for titles', () => {
    expect(shortPlace('Near Kelso')).toBe('Kelso');
    expect(tripTitleOf({ name: 'Home', approximate: false }, { name: 'Home', approximate: false })).toBe('Round trip · Home');
  });
});
