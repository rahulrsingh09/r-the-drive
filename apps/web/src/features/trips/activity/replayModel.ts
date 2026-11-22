import type { TripDetailSamples } from '@panda/types';
import type { RouteCoordinate } from './activityModel';

/** [minute from trip start, fractional route vertex index]. Never goes backwards. */
export type ReplayTimeline = Array<[number, number]>;

export interface ReplaySample {
  minute: number;
  lat: number | null;
  lng: number | null;
  battery: number | null;
}

export const REPLAY_SPEEDS = [30, 60, 240] as const;
export type ReplaySpeed = typeof REPLAY_SPEEDS[number];
/** Speed in the readout is distance over this many minutes. */
export const SPEED_WINDOW_MIN = 5;
const MAX_ROUTE_VERTICES = 400;

export function replaySamples(samples: TripDetailSamples | null | undefined): ReplaySample[] {
  if (!samples?.elapsed_s?.length) return [];
  return samples.elapsed_s.map((seconds, index) => ({
    minute: seconds / 60,
    lat: samples.lat?.[index] ?? null,
    lng: samples.lng?.[index] ?? null,
    battery: samples.battery_level?.[index] ?? null,
  }));
}

/** Keeps at most `max` vertices, always with the last one. */
export function simplifyRoute(route: RouteCoordinate[], max = MAX_ROUTE_VERTICES): RouteCoordinate[] {
  if (route.length <= max) return route;
  const step = Math.ceil(route.length / max);
  const out = route.filter((_, index) => index % step === 0);
  if (out.at(-1) !== route.at(-1)) out.push(route.at(-1)!);
  return out;
}

/** The replay route: the recorded route preview, else the positions in the samples. */
export function replayRoute(preview: RouteCoordinate[] | null | undefined, samples: ReplaySample[]): RouteCoordinate[] {
  if (preview && preview.length > 1) return simplifyRoute(preview);
  return simplifyRoute(samples.flatMap((sample) => (sample.lat != null && sample.lng != null ? [[sample.lng, sample.lat] as RouteCoordinate] : [])));
}

/**
 * Maps the timestamped GPS samples onto the route shape (port of derive.py
 * `timeline()`): each sample snaps to its nearest route vertex, and only
 * forward moves are kept.
 */
export function buildReplayTimeline(route: RouteCoordinate[], samples: ReplaySample[], totalMinutes: number): ReplayTimeline {
  if (route.length < 2) return [];
  const out: ReplayTimeline = [[0, 0]];
  let last = 0;
  for (const sample of samples) {
    if (sample.lat == null || sample.lng == null) continue;
    if (sample.minute <= 0 || (totalMinutes > 0 && sample.minute >= totalMinutes)) continue;
    const k = Math.cos((sample.lat * Math.PI) / 180);
    let best = 0;
    let bestD = Number.POSITIVE_INFINITY;
    for (let index = 0; index < route.length; index += 1) {
      const dx = (route[index]![0] - sample.lng) * k;
      const dy = route[index]![1] - sample.lat;
      const d = dx * dx + dy * dy;
      if (d < bestD) { bestD = d; best = index; }
    }
    if (best >= last) { out.push([Math.round(sample.minute * 100) / 100, best]); last = best; }
  }
  out.push([totalMinutes > 0 ? totalMinutes : out.at(-1)![0] + 1, route.length - 1]);
  return out;
}

/** Fractional route index at a minute. */
export function indexAt(timeline: ReplayTimeline, minute: number) {
  if (timeline.length < 2) return 0;
  if (minute <= timeline[0]![0]) return timeline[0]![1];
  for (let index = 1; index < timeline.length; index += 1) {
    if (timeline[index]![0] >= minute) {
      const [a, ia] = timeline[index - 1]!;
      const [b, ib] = timeline[index]!;
      return ia + ((ib - ia) * (minute - a)) / ((b - a) || 1);
    }
  }
  return timeline.at(-1)![1];
}

/** Cumulative route length per vertex, in the unit of the projected coordinates. */
export function routeLengths(route: RouteCoordinate[]) {
  const lengths = [0];
  for (let index = 1; index < route.length; index += 1) {
    const [a, b] = route[index - 1]!;
    const [c, d] = route[index]!;
    const k = Math.cos((b * Math.PI) / 180);
    lengths.push(lengths[index - 1]! + Math.hypot((c - a) * k, d - b));
  }
  return lengths;
}

/** Linear interpolation of a [minute, value] series. */
export function valueAt(series: Array<[number, number]>, minute: number): number | null {
  if (!series.length) return null;
  if (minute <= series[0]![0]) return series[0]![1];
  for (let index = 1; index < series.length; index += 1) {
    if (series[index]![0] >= minute) {
      const [a, va] = series[index - 1]!;
      const [b, vb] = series[index]!;
      return va + ((vb - va) * (minute - a)) / ((b - a) || 1);
    }
  }
  return series.at(-1)![1];
}

export interface ReplayModel {
  route: RouteCoordinate[];
  timeline: ReplayTimeline;
  totalMinutes: number;
  battery: Array<[number, number]>;
  /** Trip miles at a fractional route index. */
  milesAt: (fraction: number) => number;
}

export function buildReplayModel(preview: RouteCoordinate[] | null | undefined, samples: ReplaySample[], totalMinutes: number, tripMiles: number): ReplayModel {
  const route = replayRoute(preview, samples);
  const timeline = buildReplayTimeline(route, samples, totalMinutes);
  const lengths = routeLengths(route);
  const scale = tripMiles / (lengths.at(-1) || 1);
  return {
    route, timeline, totalMinutes,
    battery: samples.flatMap((sample) => (sample.battery != null ? [[sample.minute, sample.battery] as [number, number]] : [])),
    milesAt: (fraction) => {
      if (route.length < 2) return 0;
      const index = Math.max(0, Math.min(lengths.length - 1, Math.floor(fraction)));
      const next = Math.min(lengths.length - 1, index + 1);
      return (lengths[index]! + (lengths[next]! - lengths[index]!) * (fraction - index)) * scale;
    },
  };
}

export interface ReplayReading {
  fraction: number;
  miles: number;
  battery: number | null;
  /** Distance over the last 5 minutes; null at the ends. */
  mph: number | null;
}

export function readingAt(model: ReplayModel, minute: number): ReplayReading {
  const fraction = Math.min(indexAt(model.timeline, minute), Math.max(0, model.route.length - 1));
  const earlier = Math.max(0, minute - SPEED_WINDOW_MIN);
  const hours = (minute - earlier) / 60;
  const mph = minute > 0 && minute < model.totalMinutes && hours > 0
    ? Math.max(0, (model.milesAt(fraction) - model.milesAt(indexAt(model.timeline, earlier))) / hours) : null;
  return { fraction, miles: model.milesAt(fraction), battery: valueAt(model.battery, minute), mph };
}
