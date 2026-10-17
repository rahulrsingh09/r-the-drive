import type { TripDetailSamples } from '@panda/types';
import { parseISO } from 'date-fns';

export interface TimelinePoint {
  elapsed_s: number; ts: string; speed_mph: number | null; power_kw: number | null;
  regen_kw: number | null; estimated_net_power_kw: number | null; battery_level: number | null;
  outside_temp_c: number | null; cabin_temp_c: number | null; driver_temp_c: number | null;
  hvac_active: boolean | null; tire_fl_psi: number | null; tire_fr_psi: number | null;
  tire_rl_psi: number | null; tire_rr_psi: number | null; altitude_m: number | null;
  lat: number | null; lng: number | null;
}
const columns = ['speed_mph', 'power_kw', 'estimated_net_power_kw', 'battery_level', 'outside_temp_c', 'cabin_temp_c', 'driver_temp_c', 'hvac_active', 'tire_fl_psi', 'tire_fr_psi', 'tire_rl_psi', 'tire_rr_psi', 'altitude_m', 'lat', 'lng'] as const;

export function buildTripTimeline(start: string | undefined, samples: TripDetailSamples | undefined) {
  const timeline: TimelinePoint[] = [];
  const chartPoints: Array<{ ts: number }> = [];
  const mapTrack: Array<{ lat: number; lng: number; sampleIndex: number }> = [];
  const coverage = { directPowerSamples: 0, estimatedPowerSamples: 0, tempSamples: 0, tireSamples: 0 };
  const epoch = start ? parseISO(start).getTime() : NaN;
  const elapsed = samples && Number.isFinite(epoch) ? samples.elapsed_s : [];
  for (let index = 0; index < Math.max(elapsed.length, samples?.lat.length ?? 0); index++) {
    const lat = samples?.lat[index], lng = samples?.lng[index];
    if (index < (samples?.lat.length ?? 0) && lat != null && lng != null) mapTrack.push({ lat, lng, sampleIndex: index });
    if (index >= elapsed.length || !samples) continue;
    const elapsed_s = elapsed[index]!;
    const values = Object.fromEntries(columns.map((key) => [key, samples[key]?.[index] ?? null])) as Omit<TimelinePoint, 'elapsed_s' | 'ts' | 'regen_kw'>;
    const regen = samples.regen_power_kw[index];
    const point = { ...values, elapsed_s, ts: new Date(epoch + elapsed_s * 1000).toISOString(), regen_kw: regen == null ? null : -Math.abs(regen) };
    timeline.push(point);
    chartPoints.push({ ts: elapsed_s });
    coverage.directPowerSamples += Number(point.power_kw != null || point.regen_kw != null);
    coverage.estimatedPowerSamples += Number(point.estimated_net_power_kw != null);
    coverage.tempSamples += Number(point.outside_temp_c != null || point.cabin_temp_c != null || point.driver_temp_c != null);
    coverage.tireSamples += Number(point.tire_fl_psi != null || point.tire_fr_psi != null || point.tire_rl_psi != null || point.tire_rr_psi != null);
  }
  return { timeline, mapTrack, chartPoints, coverage };
}
