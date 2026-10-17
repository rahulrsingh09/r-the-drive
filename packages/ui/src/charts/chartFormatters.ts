import { formatPressure, formatTemp } from '../lib/utils';

type TripMetric = 'drive' | 'temperature' | 'tires';
type ReadingKind = 'power' | 'speed' | 'temperature' | 'pressure' | 'switch';
const METRICS: Record<TripMetric, Array<{ names: string[]; label: string; kind: ReadingKind }>> = {
  drive: [
    { names: ['power_kw'], label: 'Power', kind: 'power' },
    { names: ['regen_kw'], label: 'Regen', kind: 'power' },
    { names: ['speed_mph'], label: 'Speed', kind: 'speed' },
  ],
  temperature: [
    { names: ['outside_temp_c', 'Outside Temp'], label: 'Outside', kind: 'temperature' },
    { names: ['cabin_temp_c', 'Cabin Temp'], label: 'Cabin', kind: 'temperature' },
    { names: ['driver_temp_c', 'Driver Setpoint'], label: 'Driver Set', kind: 'temperature' },
    { names: ['hvac_on', 'Climate On'], label: 'Climate', kind: 'switch' },
  ],
  tires: [
    { names: ['tire_fl_psi'], label: 'Front Left', kind: 'pressure' },
    { names: ['tire_fr_psi'], label: 'Front Right', kind: 'pressure' },
    { names: ['tire_rl_psi'], label: 'Rear Left', kind: 'pressure' },
    { names: ['tire_rr_psi'], label: 'Rear Right', kind: 'pressure' },
  ],
};

export function tripTooltipFormatter(metric: TripMetric) {
  const lookup = new Map(METRICS[metric].flatMap(entry => entry.names.map(name => [name, entry] as const)));
  return (value: number | undefined, name: string): [string, string] => {
    const entry = lookup.get(name);
    if (!entry) return [String(value), String(name)];
    const numeric = Number(value);
    let text: string;
    switch (entry.kind) {
      case 'power': text = `${numeric.toFixed(1)} kW`; break;
      case 'speed': text = `${Math.round(numeric)} mph`; break;
      case 'temperature': text = formatTemp(numeric); break;
      case 'pressure': text = formatPressure(numeric); break;
      case 'switch': text = numeric > 0 ? 'On' : 'Off'; break;
    }
    return [text, entry.label];
  };
}
