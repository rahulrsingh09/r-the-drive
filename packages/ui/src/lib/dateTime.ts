import { browserStorage } from './browserRuntime';

const DEFAULT_APP_TIMEZONE = 'UTC';
const APP_TIMEZONE_STORAGE_KEY = 'rm-app-timezone';
export const APP_TIMEZONE_CHANGE_EVENT = 'rm-app-timezone-change';

const FALLBACK_TIMEZONES = [
  'UTC',
  'America/Chicago',
  'America/Denver',
  'America/Los_Angeles',
  'America/New_York',
  'America/Phoenix',
  'America/Toronto',
  'Asia/Kolkata',
  'Asia/Singapore',
  'Asia/Tokyo',
  'Australia/Sydney',
  'Europe/Berlin',
  'Europe/London',
  'Pacific/Auckland',
];

const DATE_FIELDS = ['year', 'month', 'day', 'hour', 'minute', 'second'] as const;
const PART_OPTIONS: Intl.DateTimeFormatOptions = {
  hourCycle: 'h23', year: 'numeric', month: '2-digit', day: '2-digit',
  hour: '2-digit', minute: '2-digit', second: '2-digit',
};
const formatterCache = new Map<string, Intl.DateTimeFormat>();
function formatter(locale: string | undefined, options: Intl.DateTimeFormatOptions) {
  const key = JSON.stringify([locale, options]);
  let result = formatterCache.get(key);
  if (!result) {
    result = new Intl.DateTimeFormat(locale, options);
    formatterCache.set(key, result);
  }
  return result;
}
function isValidTimezone(value: string): boolean {
  try { formatter('en-US', { timeZone: value }); return true; } catch { return false; }
}
export function getAppTimezone(): string {
  return browserStorage(DEFAULT_APP_TIMEZONE, storage => {
    const candidate = storage.getItem(APP_TIMEZONE_STORAGE_KEY);
    return candidate && isValidTimezone(candidate) ? candidate : DEFAULT_APP_TIMEZONE;
  });
}
export function setAppTimezone(timezone: string): void {
  if (!isValidTimezone(timezone)) return;
  browserStorage(undefined, (storage, browser) => {
    storage.setItem(APP_TIMEZONE_STORAGE_KEY, timezone);
    browser.dispatchEvent(new Event(APP_TIMEZONE_CHANGE_EVENT));
  });
}
export function listAppTimezones(currentTimezone?: string): string[] {
  const supported = Intl.supportedValuesOf?.('timeZone') ?? FALLBACK_TIMEZONES;
  const unique = new Set<string>();
  for (const zone of [DEFAULT_APP_TIMEZONE, currentTimezone, ...supported]) {
    if (zone) unique.add(zone);
  }
  return [...unique];
}
export function formatTimezoneOffset(timezone: string): string {
  try {
    const parts = formatter('en-US', { timeZone: timezone, timeZoneName: 'longOffset' }).formatToParts(new Date());
    const name = parts.find(part => part.type === 'timeZoneName')?.value;
    return name && name !== 'GMT' ? `UTC${name.replace(/^GMT/, '')}` : 'UTC+00:00';
  } catch { return 'UTC offset unavailable'; }
}
export function appTimezoneOptions(currentTimezone?: string) {
  return listAppTimezones(currentTimezone).map(timezone => ({
    value: timezone, label: `${timezone} (${formatTimezoneOffset(timezone)})`,
  }));
}
export interface AppDateParts {
  year: number; month: number; day: number; hour: number; minute: number; second: number;
}
function asDate(value: Date | string | number): Date {
  return value instanceof Date ? value : new Date(value);
}
function readParts(date: Date, timezone: string): AppDateParts {
  const numeric: Record<string, number> = {};
  for (const part of formatter('en-US', { timeZone: timezone, ...PART_OPTIONS }).formatToParts(date)) {
    if ((DATE_FIELDS as readonly string[]).includes(part.type)) numeric[part.type] = Number(part.value);
  }
  return { year: numeric.year ?? 0, month: numeric.month ?? 1, day: numeric.day ?? 1,
    hour: numeric.hour ?? 0, minute: numeric.minute ?? 0, second: numeric.second ?? 0 };
}
export function getAppDateParts(value: Date | string | number): AppDateParts | null {
  const date = asDate(value);
  return Number.isNaN(date.getTime()) ? null : readParts(date, getAppTimezone());
}
function wallTime(parts: AppDateParts): number {
  return Date.UTC(parts.year, parts.month - 1, parts.day, parts.hour, parts.minute, parts.second);
}
export function appDatePartsToDate(parts: AppDateParts): Date {
  const zone = getAppTimezone();
  const target = wallTime(parts);
  let candidate = target;
  // Two projections preserve the established choice at gaps and repeated hours.
  for (let pass = 0; pass < 2; pass += 1) {
    const projected = wallTime(readParts(new Date(candidate), zone));
    candidate += target - projected;
  }
  return new Date(candidate);
}
function dayBoundary(value: Date | string | number, nextDay: boolean): Date {
  const parts = getAppDateParts(value);
  if (!parts) return new Date(NaN);
  const midnight = appDatePartsToDate({ ...parts, hour: 0, minute: 0, second: 0 });
  if (!nextDay) return midnight;
  const projected = getAppDateParts(midnight);
  if (!projected) return new Date(NaN);
  return new Date(appDatePartsToDate({ ...projected, day: projected.day + 1, hour: 0, minute: 0, second: 0 }).getTime() - 1);
}
export function startOfAppDay(value: Date | string | number): Date {
  return dayBoundary(value, false);
}
export function endOfAppDay(value: Date | string | number): Date {
  return dayBoundary(value, true);
}
export function shiftAppCalendarDays(value: Date | string | number, days: number): Date {
  const parts = getAppDateParts(value);
  if (!parts) return new Date(NaN);
  const calendar = new Date(Date.UTC(parts.year, parts.month - 1, parts.day + days, 12));
  return appDatePartsToDate(Object.assign(parts, {
    year: calendar.getUTCFullYear(), month: calendar.getUTCMonth() + 1, day: calendar.getUTCDate(),
  }));
}
const DATE_FORMAT: Intl.DateTimeFormatOptions = { year: 'numeric', month: 'short', day: 'numeric' };
const TIME_FORMAT: Intl.DateTimeFormatOptions = { hour: 'numeric', minute: '2-digit' };
function display(value: Date | string | number, defaults: Intl.DateTimeFormatOptions, options: Intl.DateTimeFormatOptions): string {
  const date = asDate(value);
  return Number.isNaN(date.getTime()) ? 'Invalid date'
    : formatter(undefined, { ...defaults, ...options, timeZone: getAppTimezone() }).format(date);
}
export function formatAppDateTime(value: Date | string | number, options: Intl.DateTimeFormatOptions = {}): string {
  return display(value, { ...DATE_FORMAT, ...TIME_FORMAT }, options);
}
export function formatAppDate(value: Date | string | number, options: Intl.DateTimeFormatOptions = {}): string {
  return display(value, DATE_FORMAT, options);
}
export function formatAppTime(value: Date | string | number, options: Intl.DateTimeFormatOptions = {}): string {
  return display(value, TIME_FORMAT, options);
}
export function formatAppCalendarDate(value: string): string {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(value)) return 'Invalid date';
  const [year, month, day] = value.split('-').map(Number) as [number, number, number];
  return formatter(undefined, { dateStyle: 'medium', timeZone: 'UTC' }).format(new Date(Date.UTC(year, month - 1, day, 12)));
}
