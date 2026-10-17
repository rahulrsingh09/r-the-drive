import { statusLabel } from './statusLabel';
import { formatPressure } from './utils';

export const DEFAULT_TARGET_TIRE_PRESSURE_PSI = 40;

type TireHealthTone = 'neutral' | 'success' | 'warning' | 'danger';
export type TireWheel = {
  position: 'fl' | 'fr' | 'rl' | 'rr';
  label: string;
  psi: number | null;
  status: string | null;
  valid: boolean | null;
};
export type TireHealthLegendEntry = {
  tone: Exclude<TireHealthTone, 'neutral'>;
  label: string;
  rangeLabel: string;
  detail: string;
};

type TireValues = Partial<Record<
  | 'tire_fl_psi'
  | 'tire_fr_psi'
  | 'tire_rl_psi'
  | 'tire_rr_psi'
  | 'tire_fl_status'
  | 'tire_fr_status'
  | 'tire_rl_status'
  | 'tire_rr_status'
  | 'tire_fl_valid'
  | 'tire_fr_valid'
  | 'tire_rl_valid'
  | 'tire_rr_valid',
  number | string | boolean | null | undefined
>>;

const TIRE_WHEEL_DEFINITIONS = [
  ['fl', 'Front Left'],
  ['fr', 'Front Right'],
  ['rl', 'Rear Left'],
  ['rr', 'Rear Right'],
] as const;

export function normalizeTireWheels(status: TireValues | null | undefined, tires: TireValues | null | undefined): TireWheel[] {
  const fallback = { ...tires };
  for (const [key, value] of Object.entries(status ?? {})) {
    if (value != null) fallback[key as keyof TireValues] = value;
  }
  return TIRE_WHEEL_DEFINITIONS.map(([position, label]) => ({
    position, label,
    psi: (fallback[`tire_${position}_psi`] ?? null) as number | null,
    status: (fallback[`tire_${position}_status`] ?? null) as string | null,
    valid: (status?.[`tire_${position}_valid`] ?? null) as boolean | null,
  }));
}
export function formatTireLabel(psi: number | null | undefined, status?: string | null | undefined) {
  return psi == null ? prettifyTireStatus(status) : formatPressure(psi);
}
const TONE_BORDERS = {
  neutral: 'border-border', success: 'border-status-positive/70',
  warning: 'border-status-warning/70', danger: 'border-status-danger/70',
};
const HEALTH_BANDS = [
  { tone: 'success', maximumDeficit: 2, label: 'Green', detail: 'Within 2 PSI of target or above it' },
  { tone: 'warning', maximumDeficit: 5, label: 'Yellow', detail: '3-5 PSI below target' },
  { tone: 'danger', maximumDeficit: Infinity, label: 'Red', detail: '6+ PSI below target' },
] as const;
export function getTireHealthTone({ psi, status, targetPsi }: { psi: number | null | undefined; status?: string | null | undefined; targetPsi?: number | null | undefined }): TireHealthTone {
  const sensor = status?.trim().toLowerCase() ?? '';
  if (sensor.includes('invalid_sensor')) return 'neutral';
  if (psi == null) return /(low|high|warn|critical|fault)/i.test(sensor) ? 'warning' : 'neutral';
  const deficit = Math.round(targetPsi ?? DEFAULT_TARGET_TIRE_PRESSURE_PSI) - Math.round(psi);
  return HEALTH_BANDS.find(band => deficit <= band.maximumDeficit)?.tone ?? 'danger';
}
export function summarizeTireHealth(wheels: TireWheel[], targetPsi?: number | null | undefined) {
  const result: { tones: TireHealthTone[]; hasInvalidSensor: boolean; readings: number[]; tone: TireHealthTone } = { tones: [], hasInvalidSensor: false, readings: [], tone: 'neutral' };
  const ranks = { neutral: 0, success: 1, warning: 2, danger: 3 };
  for (const wheel of wheels) {
    const tone = getTireHealthTone({ psi: wheel.psi, status: wheel.status, targetPsi });
    result.tones.push(tone);
    result.hasInvalidSensor ||= wheel.valid === false;
    if (typeof wheel.psi === 'number') result.readings.push(wheel.psi);
    if (ranks[tone] > ranks[result.tone]) result.tone = tone;
  }
  return result;
}
export function tireHealthBorderClass(tone: TireHealthTone) {
  return TONE_BORDERS[tone] ?? TONE_BORDERS.neutral;
}
export function getTireHealthLegend(targetPsi?: number | null | undefined): TireHealthLegendEntry[] {
  const target = Math.round(targetPsi ?? DEFAULT_TARGET_TIRE_PRESSURE_PSI);
  return HEALTH_BANDS.map((band, index) => ({
    tone: band.tone, label: band.label, detail: band.detail,
    rangeLabel: index === 0 ? `${Math.ceil(target - 2)}+ psi`
      : index === 1 ? `${Math.ceil(target - 5)}-${Math.ceil(target - 3)} psi`
      : `<=${Math.ceil(target - 6)} psi`,
  }));
}
function prettifyTireStatus(value: string | null | undefined) {
  return statusLabel(value || '-', /_/, 'chrgr_sts_');
}
