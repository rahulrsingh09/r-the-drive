import { describe, expect, it } from 'vitest';
import { buildReplayModel, buildReplayTimeline, indexAt, readingAt, simplifyRoute, valueAt, type ReplaySample } from './replayModel';
import type { RouteCoordinate } from './activityModel';

const route: RouteCoordinate[] = Array.from({ length: 11 }, (_, index) => [-122.2, 47.6 + index * 0.01]);
const samples: ReplaySample[] = [
  { minute: 2, lat: 47.62, lng: -122.2, battery: 79 },
  { minute: 4, lat: 47.61, lng: -122.2, battery: 78 }, // a backward GPS jump is ignored
  { minute: 6, lat: 47.66, lng: -122.2, battery: 77 },
  { minute: 9, lat: 47.69, lng: -122.2, battery: 76 },
];

describe('replay timeline', () => {
  it('maps timestamped positions onto route vertices and never goes backwards', () => {
    expect(buildReplayTimeline(route, samples, 10)).toEqual([[0, 0], [2, 2], [6, 6], [9, 9], [10, 10]]);
  });

  it('interpolates the route index and values at a minute', () => {
    const timeline = buildReplayTimeline(route, samples, 10);
    expect(indexAt(timeline, 4)).toBe(4);
    expect(valueAt([[0, 80], [10, 70]], 5)).toBe(75);
  });

  it('reads distance, battery and speed as distance over 5 minutes', () => {
    const model = buildReplayModel(route, [{ minute: 0, lat: 47.6, lng: -122.2, battery: 80 }, ...samples], 10, 10);
    const reading = readingAt(model, 6);
    expect(reading.miles).toBeCloseTo(6);
    expect(reading.battery).toBe(77);
    expect(reading.mph).toBeCloseTo(60);
    expect(readingAt(model, 10).mph).toBeNull();
  });

  it('falls back to sample positions when there is no route preview, and caps vertices', () => {
    expect(buildReplayModel(null, samples, 10, 1).route).toHaveLength(4);
    expect(simplifyRoute(Array.from({ length: 1000 }, (_, index) => [index, index] as RouteCoordinate), 100).at(-1)).toEqual([999, 999]);
  });
});
