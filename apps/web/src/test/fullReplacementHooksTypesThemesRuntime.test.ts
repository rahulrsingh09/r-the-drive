import { afterEach, describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { registryManifest, resolveTheme, stableThemeJson, themeHash, validateThemeOverride } from '../../../../packages/themes/src/index';
import { decodeGetChargeCurve, decodeGetTripDetailSeries, decodeGetSpeedProfile, decodeGetBatteryMileage, decodeGetChargingSummary } from '../../../../packages/hooks/src/api/normalization';
import { resolveVehicleArtwork } from '../../../../packages/hooks/src/vehicleArtworkFallback';
import type { VehicleImages } from '@panda/types';
import { withSessionLock } from '../../../../packages/hooks/src/api/sessionLock';
import { AuthenticatedTransport } from '../../../../packages/hooks/src/api/transport';

afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); vi.useRealTimers(); });

describe('fixed theme output and override isolation', () => {
  it('matches every shipped theme token, paint, alias and registry hash', () => {
    const expected = JSON.parse(readFileSync(resolve(process.cwd(), '../api/src/themes/builtins.generated.json'), 'utf8'));
    expect(registryManifest()).toEqual(expected);
    expect(expected.registryHash).toBe('4a9614d271d9ef24');
  });
  it('keeps partial overrides isolated and preserves ordered validation diagnostics', () => {
    const original = resolveTheme({ theme: 'rad' });
    const override = resolveTheme({ theme: 'rad', tokens: { accent: { light: '#01020304' } }, brandPaints: { mark: { dark: '#123456' } }, series: { 'series-16': { light: '#654321' } } });
    expect(override.tokens.dark).toEqual(original.tokens.dark);
    expect(override.tokens.light.accent).toBe('#01020304');
    expect(override.brandPaints.mark.light).toBe(original.brandPaints.mark.light);
    expect(resolveTheme({ theme: 'rad' })).toEqual(original);
    expect(validateThemeOverride({ extra: true, theme: 'missing', tokens: { invalid: { wrong: 'red' } }, series: { 'series-17': { dark: 'url(x)' } }, brandPaints: { missing: { light: 'var(x)' } } } as never)).toEqual([
      'Unsupported theme property: extra', 'Unknown built-in theme: missing', 'Token is not customizable: invalid', 'Invalid custom color for invalid.wrong', 'Unknown series slot: series-17', 'Invalid custom color for series-17.dark', 'Unknown brand paint: missing', 'Invalid custom color for brandPaints.missing.light',
    ]);
  });
  it('serializes nested objects and sparse arrays with stable ordering', () => {
    const sparse = new Array(4); sparse[0] = 1; sparse[2] = undefined; sparse[3] = { z: false, a: 'é' };
    expect(stableThemeJson({ z: sparse, a: null })).toBe('{"a":null,"z":[1,,,{"a":"é","z":false}]}');
    expect(stableThemeJson({ b: undefined, a: 1 })).toBe('{"a":1,"b":undefined}');
    expect(stableThemeJson({ a: Symbol('test'), b: () => undefined })).toBe('{"a":undefined,"b":undefined}');
    expect(stableThemeJson(Symbol('test'))).toBeUndefined();
    expect(themeHash(resolveTheme())).toBe(themeHash(resolveTheme()));
  });
});

describe('wire projection contracts', () => {
  it('preserves numeric zero, aliases and omission of optional fields', () => {
    expect(decodeGetChargeCurve([{ soc_pct: '0', soc: 50, power_kw: Infinity, charge_rate_kw: '7.2', sample_source: null }, {}])).toEqual([
      { minutes_elapsed: null, soc_pct: 0, power_kw: 7.2 }, { minutes_elapsed: null, soc_pct: 0, power_kw: 0 },
    ]);
    expect(decodeGetSpeedProfile([{}, { elapsed_s: '0', value: '0' } as never])).toEqual([{ elapsed_s: 0, speed_mph: 0 }, { elapsed_s: 0, speed_mph: 0 }]);
    vi.useFakeTimers(); vi.setSystemTime(new Date('2026-01-01T00:00:00Z'));
    expect(decodeGetBatteryMileage([{}])[0]?.ts).toBe('2026-01-01T00:00:00.000Z');
    const points = decodeGetTripDetailSeries([{ power_source: 'direct', hvac_active: false, speed_mph: '9' }, { power_source: 'invalid' }]);
    expect(points[0]).toMatchObject({ power_source: 'direct', hvac_active: false, speed_mph: 9 });
    expect(points[1]).not.toHaveProperty('power_source');
    expect(() => decodeGetTripDetailSeries([null] as never)).toThrow('Expected response rows');
  });
  it('keeps legacy summary arithmetic, null defaults and independent trip tag flags', async () => {
    expect(decodeGetChargingSummary({ ac_kwh: 0, by_type: { ac_kwh: 99, ac_l2_kwh: 3, dc_kwh: 7 }, total_kwh: 10, weekly: [{ week_start: 'day', sessions: 2, kwh: 4 }] })).toMatchObject({ ac_kwh: 3, dc_kwh: 7, total_energy_kwh: 10, total_cost_usd: null, weekly: [{ week_start: 'day', sessions: 2, energy_kwh: 4 }] });
    const fetch = vi.spyOn(globalThis, 'fetch').mockResolvedValue(new Response('{}'));
    const client = new AuthenticatedTransport();
    await client.getTripMap('v', null, null, '', false, { tagIds: [''], untagged: true });
    expect(String(fetch.mock.calls[0]?.[0])).toBe('/v1/trips/map?vehicle_id=v&lifetime=true&tag_ids=&tag_match=all&untagged=1');
  });
});

describe('artwork ranking', () => {
  it('prefers health roles before mode while keeping typed side ahead of plain side', () => {
    const images = { side: { light: '/typed-light', dark: '/typed-dark' }, all: [
      { placement: 'side', design: 'dark', url: '/side' },
      { placement: 'health-hero-fallback', design: 'light', url: '/fallback' },
      { placement: 'three_quarter', design: 'light', url: '/quarter' },
      { placement: 'health-hero', design: 'dark', url: '/hero' },
    ] } as VehicleImages;
    expect(resolveVehicleArtwork(images, 'R1S', 'health')).toMatchObject({ light: '/hero', dark: '/hero' });
    expect(resolveVehicleArtwork({ ...images, all: images.all.slice(0, 2) }, 'R1S', 'health')).toMatchObject({ light: '/typed-light', dark: '/typed-dark' });
  });
  it('keeps the first semantic charging image and packaged model path', () => {
    const images = { all: [{ placement: 'side', design: 'light', url: '/open', metadata: { description: 'port open' } }, { placement: 'side_charging', design: 'light', url: '/later' }] } as VehicleImages;
    expect(resolveVehicleArtwork(images, 'R 2', 'charging')).toEqual({ light: '/open', dark: '/open', fallback: '/vehicle-images/fallbacks/r2s/charging.webp' });
  });
});

describe('session coordination fallbacks', () => {
  it('serializes local waiters and releases after a rejection', async () => {
    vi.stubGlobal('indexedDB', undefined);
    vi.stubGlobal('navigator', { userAgent: 'jsdom' });
    const order: string[] = [];
    let release!: () => void;
    const first = withSessionLock(async () => { order.push('first'); await new Promise<void>((resolve) => { release = resolve; }); throw new Error('synthetic'); });
    const second = withSessionLock(async () => { order.push('second'); return 2; });
    await Promise.resolve();
    expect(order).toEqual(['first']);
    release();
    await expect(first).rejects.toThrow('synthetic');
    await expect(second).resolves.toBe(2);
    expect(order).toEqual(['first', 'second']);
  });
  it('fails closed in a browser lacking both cross-tab lock mechanisms', async () => {
    vi.stubGlobal('indexedDB', undefined);
    vi.stubGlobal('navigator', { userAgent: 'synthetic-browser' });
    const operation = vi.fn();
    await expect(withSessionLock(operation)).rejects.toMatchObject({ code: 'NETWORK_ERROR' });
    expect(operation).not.toHaveBeenCalled();
  });
});
