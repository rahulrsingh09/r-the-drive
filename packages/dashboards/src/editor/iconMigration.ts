/** Map legacy short keys (pre-Iconify icon picker) to Iconify IDs. */
const LEGACY_ICON_MAP = new Map(Object.entries({
  activity: 'lucide:activity',
  battery: 'lucide:battery',
  bolt: 'lucide:bolt',
  calendar: 'lucide:calendar-days',
  clock: 'lucide:clock-3',
  gauge: 'lucide:gauge',
  map: 'lucide:map',
  route: 'lucide:route',
  thermometer: 'lucide:thermometer',
  zap: 'lucide:zap',
}));

/** Resolve any sensor icon option (legacy short key OR iconify id) to an iconify id. */
export function resolveIconId(value: string | null | undefined, fallback = 'lucide:activity'): string {
  if (!value || typeof value !== 'string') return fallback;
  if (value.includes(':')) return value;
  return LEGACY_ICON_MAP.get(value) ?? fallback;
}

export function isIconifyId(value: unknown): value is string {
  return typeof value === 'string' && value.includes(':') && value.length >= 3;
}
