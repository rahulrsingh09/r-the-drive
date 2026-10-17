import { afterEach, describe, expect, it, vi } from 'vitest';
import { isVehicleCharging, resolveVehicleGateCapability } from '@panda/types';
import {
  decodeGetChargeSession,
  decodeListChargeSessions,
  decodeListTrips,
  decodeUpdateChargeSession,
} from '../../../../packages/hooks/src/api/normalization';
import { AuthenticatedTransport, setApiBaseUrl } from '../../../../packages/hooks/src/api/transport';
import { isLocalArtworkSource } from '../../../../packages/hooks/src/outboundPolicy';

afterEach(() => {
  vi.restoreAllMocks();
  setApiBaseUrl(undefined);
});

describe('retained capability contracts', () => {
  it.each([
    [undefined, null],
    [null, null],
    ['', null],
    ['unknown', null],
    ['  rivian r1t quad  ', 'tailgate'],
    ['R1S', 'liftgate'],
    ['r2s', 'liftgate'],
    ['R2 Performance', 'liftgate'],
    ['R1S / R1T', 'tailgate'],
    ['R2 / R1S / R1T', 'tailgate'],
    ['prefixR1Tsuffix', 'tailgate'],
    ['R 1 T', null],
  ] as const)('resolves %j with fixed model priority', (model, expected) => {
    expect(resolveVehicleGateCapability(model)).toBe(expected);
  });

  it('preserves the charging truth table and case rules', () => {
    for (let mask = 0; mask < 8; mask++) {
      expect(isVehicleCharging({
        power_state: mask & 1 ? 'charging' : 'sleep',
        charger_state: mask & 2 ? 'Charging' : 'Disconnected',
        charger_status: mask & 4 ? 'chrgr_sts_connected_charging' : 'unknown',
      })).toBe(mask !== 0);
    }
    expect(isVehicleCharging(null)).toBe(false);
    expect(isVehicleCharging(undefined)).toBe(false);
    expect(isVehicleCharging({ charger_state: 'cHaRgInG' } as never)).toBe(true);
    expect(isVehicleCharging({ power_state: 'CHARGING', charger_state: ' charging ' } as never)).toBe(false);
  });
});

describe('page decoding contracts', () => {
  const pageDecoders = [decodeListTrips, decodeListChargeSessions];

  it.each(pageDecoders)('preserves canonical precedence and source immutability', (decode) => {
    const items = Object.freeze([{ id: 'canonical' }]);
    const response = Object.freeze({
      items, data: [{ id: 'legacy' }], total: 0, page: 0, per_page: 0, limit: 50, offset: 100,
    });
    const result = decode(response as never, 3, 7);
    expect(result).toMatchObject({ items: [{ id: 'canonical' }], total: 0, page: 0, per_page: 0 });
    expect(result.items).not.toBe(items);
    expect(items).toEqual([{ id: 'canonical' }]);
    expect(result).not.toHaveProperty('data');
    expect(result).not.toHaveProperty('offset');
  });

  it.each(pageDecoders)('uses legacy limit and offset, then request defaults', (decode) => {
    expect(decode({ data: [{ id: 'legacy' }], limit: 4, offset: 9, total: null } as never, 6, 8))
      .toMatchObject({ items: [{ id: 'legacy' }], total: 0, page: 3, per_page: 4 });
    expect(decode({ items: [], data: [{ id: 'ignored' }] } as never, 6, 8))
      .toEqual({ items: [], total: 0, page: 6, per_page: 8 });
    expect(decode({ items: null, data: null, offset: 0 } as never, 6, 8))
      .toEqual({ items: [], total: 0, page: 1, per_page: 8 });
  });

  it.each(pageDecoders)('rejects malformed collections without masking canonical errors', (decode) => {
    for (const response of [null, { items: {}, data: [] }, { items: 'bad' }, { data: {} }]) {
      expect(() => decode(response as never, 1, 10)).toThrow();
    }
  });

  it.each([decodeGetChargeSession, decodeUpdateChargeSession])('preserves charge envelope presence', (decode) => {
    expect(decode({ session: { id: 'nested', kwh_added: '4' }, id: 'ignored' }))
      .toMatchObject({ id: 'nested', energy_added_kwh: 4 });
    expect(decode({ id: 'bare' })).toMatchObject({ id: 'bare' });
    expect(decode(Object.create({ session: { id: 'inherited' } }))).toMatchObject({ id: 'inherited' });
    expect(() => decode({ id: 'ignored', session: null })).toThrow('missing id');
    expect(() => decode({ id: 'ignored', session: undefined })).toThrow('missing id');
  });
});

describe('artwork policy boundaries', () => {
  it.each([
    [null, false], [undefined, false], ['', false], ['/', true], ['/a', true],
    ['//elsewhere.invalid/a', false], ['/a\\b', false], ['/a b', false],
    ['/a\nb', false], ['https://elsewhere.invalid/a', false], ['/a?size=1#image', true],
    ['/a%20b', true],
  ] as const)('classifies local artwork %j', (source, expected) => {
    expect(isLocalArtworkSource(source)).toBe(expected);
  });

  it.each([
    '/v1/vehicle-image-cache/../../auth/login',
    '/v1/vehicle-image-cache/%2e%2e/%2e%2e/auth/login',
    '/v1/vehicle-image-cache/..\\..\\auth/login',
    '/v1/vehicle-image-cache/\t../../auth/login',
  ])('rejects a browser-normalized escape before fetching: %s', async (path) => {
    const fetch = vi.spyOn(globalThis, 'fetch').mockResolvedValue(new Response('synthetic'));
    const client = new AuthenticatedTransport();
    client.setToken('synthetic-access');
    setApiBaseUrl('');
    await expect(client.authenticatedAsset(path)).rejects.toThrow('only accepts');
    expect(fetch).not.toHaveBeenCalled();
  });

  it('preserves safe asset bytes, headers, queries, and in-prefix normalization', async () => {
    const response = new Response('synthetic-artwork');
    const fetch = vi.spyOn(globalThis, 'fetch').mockResolvedValue(response);
    const client = new AuthenticatedTransport();
    client.setToken('synthetic-access');
    setApiBaseUrl('');
    const path = '/v1/vehicle-image-cache/a/../vehicle/artwork.webp?size=small#image';
    await expect(client.authenticatedAsset(path)).resolves.toBe(response);
    expect(fetch).toHaveBeenCalledExactlyOnceWith(path, expect.objectContaining({
      method: 'GET',
      credentials: 'include',
      headers: expect.objectContaining({ Authorization: 'Bearer synthetic-access' }),
    }));
  });
});
