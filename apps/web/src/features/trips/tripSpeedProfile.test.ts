import { describe, expect, it } from 'vitest';
import { buildTripSpeedProfile } from './tripSpeedProfile';

const point = (elapsed_s: number, lng: number | null = null, speed_mph: number | null = null) => ({
  elapsed_s, speed_mph, lat: lng == null ? null : 40, lng,
});
const total = (profile: ReturnType<typeof buildTripSpeedProfile>) =>
  profile.bins.reduce((sum, bin) => sum + bin.duration_seconds, 0);

describe('trip speed sources and coverage', () => {
  it('weights recorded buckets by elapsed time and clips to the trip end', () => {
    const result = buildTripSpeedProfile([point(-1, null, 30), point(1, null, 30), point(3, null, 40)], 4, 2);
    expect(result.recordedSeconds).toBe(4);
    expect(result.estimatedSeconds).toBe(0);
    expect(result.bins.find(bin => bin.min === 30)?.duration_seconds).toBe(3);
    expect(total(result)).toBe(4);
  });

  it('counts a minute of GPS history as a minute, not as one small API bucket', () => {
    const points = [point(0, -105), point(30), point(60, -104.99)];
    const original = structuredClone(points);
    const result = buildTripSpeedProfile(points, 90, 2);
    expect(result.estimatedSeconds).toBe(60);
    expect(result.missingSeconds).toBe(30);
    expect(result.coverage).toBeCloseTo(2 / 3);
    expect(result.medianGpsSeconds).toBe(60);
    expect(result.estimates[1]?.mph).toBeGreaterThan(30);
    expect(result.estimates[2]?.mph).toBeLessThan(35);
    expect(points).toEqual(original);
    expect(result.recordedValues).toEqual([null, null, null]);
  });

  it('keeps reported zero and removes its interval from the GPS estimate', () => {
    const result = buildTripSpeedProfile([point(0, -105), point(30, null, 0), point(60, -104.99)], 60, 10);
    expect(result.recordedSeconds).toBe(10);
    expect(result.estimatedSeconds).toBe(50);
    expect(result.estimates[1]).toBeNull();
    expect(result.recordedValues[1]).toBe(0);
    expect(total(result)).toBe(60);
    expect(result.bins[0]?.recorded_seconds).toBe(10);
  });

  it('does not extend direct speed across a telemetry gap', () => {
    const result = buildTripSpeedProfile([point(0, null, 40), point(300, null, 50)], 600, 2);
    expect(result.recordedSeconds).toBe(4);
    expect(result.missingSeconds).toBe(596);
  });

  it.each([
    [point(0, -105), point(120, -104.99)],
    [point(0, -105), point(2, -104.99)],
    [point(0, -105), point(60, -104)],
    [point(0, -105), point(60, -105)],
    [point(0, -105), point(60, -104.99999)],
    [point(0, -105), { ...point(30, -104.99), lat: 200 }, point(60, -104.98)],
    [point(0, -105), point(-1, -104.99), point(60, -104.98)],
  ])('rejects unsafe GPS intervals: %j', (...points) => {
    expect(buildTripSpeedProfile(points, 120, 2).estimatedSeconds).toBe(0);
  });

  it('does not use odometer readings or invent a stopped bin for missing speed', () => {
    const points = [{ ...point(0), odometer_miles: 100 }, { ...point(60), odometer_miles: 101 }];
    const result = buildTripSpeedProfile(points, 60, 2);
    expect(result.bins).toEqual([]);
    expect(result.coverage).toBe(0);
  });

  it('bounds invalid speed and interval inputs without unbounded bins', () => {
    const result = buildTripSpeedProfile([point(0, null, Infinity), point(10, null, -3), point(20, null, NaN)], 60, 0);
    expect(result.bins).toEqual([]);
    expect(result.recordedValues).toEqual([null, null, null]);
  });
});
