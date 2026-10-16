import { afterEach, describe, expect, it, vi } from 'vitest';
import { AuthenticatedTransport } from '../../../../packages/hooks/src/api/transport';
import {
  buildTimeframeParams,
  buildTripTagFilterParams,
  finiteNumber,
  isRecord,
  normalizeTrip,
  normalizeChargeSession,
} from '../../../../packages/hooks/src/api/normalization';

afterEach(() => vi.restoreAllMocks());

describe('legacy wire normalization', () => {
  it.each([
    [0, 0],
    [' 2.5 ', 2.5],
    ['0', 0],
    [null, undefined],
    ['', undefined],
    [' ', undefined],
    [true, undefined],
    [Infinity, undefined],
    ['NaN', undefined],
    [{}, undefined],
  ])('finiteNumber(%j) = %j', (input, expected) => {
    expect(finiteNumber(input)).toBe(expected);
  });

  it('distinguishes records from arrays and null', () => {
    expect(isRecord({})).toBe(true);
    expect(isRecord([])).toBe(false);
    expect(isRecord(null)).toBe(false);
  });

  it('honors canonical zero values before aliases and derives energy only as a fallback', () => {
    expect(
      normalizeTrip({
        distance_mi: 0,
        distance_miles: 5,
        duration_min: 0,
        duration_seconds: 120,
        efficiency_wh_mi: 0,
        efficiency_wh_per_mile: 300,
        energy_used_kwh: 0,
        energy_wh: 500,
      })
    ).toMatchObject({ distance_mi: 0, duration_min: 0, efficiency_wh_mi: 0, energy_used_kwh: 0 });
    expect(
      normalizeTrip({
        distance_miles: '10',
        duration_seconds: '120',
        efficiency_wh_per_mile: '300',
      })
    ).toMatchObject({ distance_mi: 10, duration_min: 2, energy_used_kwh: 3 });
    expect(normalizeTrip({ distance_mi: 0, efficiency_wh_mi: 300 }).energy_used_kwh).toBeNull();
    expect(
      normalizeTrip({ distance_mi: 10, efficiency_wh_mi: 300, energy_wh: 2000 }).energy_used_kwh
    ).toBe(2);
  });

  it('keeps valid coordinate pairs, labels, and tag defaults without mutating wire data', () => {
    const raw = Object.freeze({
      id: 12,
      vehicle_id: 'v',
      start_lat: 0,
      start_lng: 12,
      end_lat: 0,
      end_lng: 0,
      start_place_name: 'Home',
      end_place: '',
      tags: [{ id: 'a', name: 'Work', color_token: 'unexpected' }, { name: 'bad' }, null],
    });
    expect(normalizeTrip(raw)).toMatchObject({
      id: '12',
      start_lat: 0,
      start_lng: 12,
      end_lat: null,
      end_lng: null,
      start_place: 'Home',
      end_place: '',
      tags: [
        {
          id: 'a',
          name: 'Work',
          vehicle_id: 'v',
          color_token: 'accent',
          created_by: '',
          created_at: '',
          updated_at: '',
        },
      ],
    });
  });

  it.each([null, [], false, 'invalid'])('retains tolerant trip defaults for %j', (raw) => {
    expect(normalizeTrip(raw)).toMatchObject({
      id: '',
      distance_mi: 0,
      duration_min: 0,
      energy_used_kwh: null,
      tags: [],
    });
  });

  it('requires charging identity and preserves uppercase charger and nullable override contracts', () => {
    expect(() => normalizeChargeSession({})).toThrow('missing id');
    expect(
      normalizeChargeSession({
        id: 'c',
        charger_type: 'AC',
        location_override_mode: 'none',
        cost_override_mode: 'free',
      })
    ).toMatchObject({
      charger_type: 'AC',
      location_override_mode: 'none',
      cost_override_mode: 'free',
    });
    expect(
      normalizeChargeSession({ id: 'c', charger_type: 'ac', location_override_mode: 'invalid' })
    ).toMatchObject({ charger_type: null, location_override_mode: null });
  });

  it('preserves charging aliases, costs, location precedence, and live fields', () => {
    const result = normalizeChargeSession({
      id: 'c',
      location_name: '0.0000, 0.0000',
      is_home: true,
      energy_added_wh: '7000',
      max_charge_rate_kw: '7.5',
      duration_minutes: '80',
      cost_usd: 0,
      is_free_session: true,
      current_soc: '85',
      time_elapsed_seconds: '120',
      live_time_remaining_minutes: '5',
      current_charger_state: 'charging',
      parallax_charger_status: 'active',
      live_session_started_at: 123,
    });
    expect(result).toMatchObject({
      location_name: 'Home',
      energy_added_kwh: 7,
      peak_power_kw: 7.5,
      duration_min: 80,
      cost_usd: 0,
      is_free_session: true,
      live_soc_pct: 85,
      live_time_elapsed_seconds: 120,
      live_time_remaining_min: 5,
      live_charger_state: 'charging',
      live_charger_status: 'active',
      live_started_at: '123',
    });
    expect(
      normalizeChargeSession({ id: 'c', location_lat: '47.123456', location_lng: '-122.98765' })
        .location_name
    ).toBe('47.1235, -122.9877');
    expect(
      normalizeChargeSession({ id: 'c', location_name: ' Saved ', is_home: true }).location_name
    ).toBe('Saved');
  });

  it('preserves lifetime behavior and efficiency tag precedence', () => {
    expect(buildTimeframeParams(null, null)).toEqual({ lifetime: 'true' });
    expect(buildTimeframeParams('from', 'to', true)).toEqual({ lifetime: 'true' });
    expect(buildTimeframeParams(null, 'to')).toEqual({ to: 'to' });
    expect(
      buildTripTagFilterParams({ tagIds: ['z', 'a', 'z'], tagMatch: 'any', untagged: true })
    ).toEqual({ tag_ids: 'a,z', tag_match: 'any' });
    expect(buildTripTagFilterParams({ untagged: true })).toEqual({ untagged: 1 });
  });
});

describe('endpoint decoder boundaries', () => {
  it('handles both pagination envelopes, defaults and explicit zero totals', async () => {
    const client = new AuthenticatedTransport();
    vi.spyOn(globalThis, 'fetch')
      .mockResolvedValueOnce(
        new Response(JSON.stringify({ data: [{ id: 't' }], limit: 10, offset: 20, total: 0 }))
      )
      .mockResolvedValueOnce(
        new Response(JSON.stringify({ items: [{ id: 'c' }], page: 2, per_page: 7, total: 4 }))
      )
      .mockResolvedValueOnce(new Response('{}'));
    expect(await client.listTrips('v', null, null)).toMatchObject({
      page: 3,
      per_page: 10,
      total: 0,
      items: [{ id: 't' }],
    });
    expect(await client.listChargeSessions('v', null, null)).toMatchObject({
      page: 2,
      per_page: 7,
      total: 4,
      items: [{ id: 'c' }],
    });
    expect(await client.listTrips('v', null, null, 4, 8)).toEqual({
      page: 4,
      per_page: 8,
      total: 0,
      items: [],
    });
  });

  it('unwraps bare/nested charge responses with identical results', async () => {
    const client = new AuthenticatedTransport();
    vi.spyOn(globalThis, 'fetch')
      .mockResolvedValueOnce(new Response(JSON.stringify({ id: 'c', kwh_added: 5 })))
      .mockResolvedValueOnce(new Response(JSON.stringify({ session: { id: 'c', kwh_added: 5 } })));
    expect(await client.getChargeSession('c', 'v')).toEqual(
      await client.updateChargeSession('v', 'c', {})
    );
  });

  it.each([
    ['listVehicles', [], [null, { vehicles: 'invalid' }, { vehicles: {} }]],
    ['listTrips', ['v', null, null], [null, { items: {} }]],
    ['getChargeSession', ['c', 'v'], [{}, null]],
    ['getSpeedProfile', ['t', 'v'], [{}, [null], ['invalid']]],
    ['getTripPowerProfile', ['t', 'v'], [null, [null]]],
  ] as const)(
    'reports malformed %s responses with endpoint context',
    async (method, args, malformed) => {
      const client = new AuthenticatedTransport();
      const fetchMock = vi.spyOn(globalThis, 'fetch');
      for (const value of malformed) {
        fetchMock.mockResolvedValueOnce(
          new Response(JSON.stringify(value), { headers: { 'x-request-id': 'shape-1' } })
        );
        const operation = client[method] as (...args: unknown[]) => Promise<unknown>;
        await expect(operation.apply(client, [...args])).rejects.toMatchObject({
          code: 'INVALID_RESPONSE',
          detail: { requestId: 'shape-1', method: 'GET' },
        });
      }
    }
  );

  it('retains empty list defaults for absent/null envelope fields', async () => {
    const client = new AuthenticatedTransport();
    vi.spyOn(globalThis, 'fetch')
      .mockResolvedValueOnce(new Response('{}'))
      .mockResolvedValueOnce(new Response('{"vehicles":null}'));
    expect(await client.listVehicles()).toEqual([]);
    expect(await client.listVehicles()).toEqual([]);
  });

  it('retains different trip/efficiency tag semantics and optional telemetry parameters', async () => {
    const client = new AuthenticatedTransport();
    const fetchMock = vi
      .spyOn(globalThis, 'fetch')
      .mockImplementation(async () => new Response('{}'));
    await client.getTripMap('v', null, null, '   ', false, {
      tagIds: ['b', 'a', 'b'],
      untagged: true,
    });
    await client.getEfficiencyTrend('v', null, null, false, {
      tagIds: ['b', 'a', 'b'],
      untagged: true,
    });
    await client.getRawTelemetry('v', 0);
    await client.getTelemetryLanes('v', { max_points: 0 });
    expect(fetchMock.mock.calls.map(([url]) => String(url))).toEqual([
      '/v1/trips/map?vehicle_id=v&lifetime=true&tag_ids=a%2Cb&tag_match=all&untagged=1',
      '/v1/efficiency/trend?vehicle_id=v&lifetime=true&tag_ids=a%2Cb&tag_match=all',
      '/v1/vehicles/v/raw-data?',
      '/v1/vehicles/v/telemetry/lanes?',
    ]);
  });
});
