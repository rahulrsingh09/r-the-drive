import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { isDeepStrictEqual } from 'node:util';
import {
  AuthenticatedTransport,
  setApiBaseUrl,
} from '../../../../packages/hooks/src/api/transport';
import * as clients from '../../../../packages/hooks/src/api/clients';
import cases from './rewriteApiClient.cases.json';

// Recorded against 8e32427 before replacement; use only synthetic data.
const row = {
  id: 'record-1',
  vehicle_id: 'vehicle-1',
  ts: '2026-01-01T00:00:00Z',
  started_at: '2026-01-01T00:00:00Z',
  distance_miles: 12,
  duration_seconds: 1200,
  efficiency_wh_per_mile: 320,
  kwh_added: 9,
  max_charge_rate_kw: 7,
  soc: 55,
  value: 8,
  avg_wh_per_mi: 320,
  drive_mode: 'all_purpose',
  trip_count: 2,
};
const arrayMethods = new Set([
  'getBatteryMileage',
  'getSpeedProfile',
  'getTripPowerProfile',
  'getTripDetailSeries',
  'getChargeCurve',
  'getChargeCurveAnalysis',
  'getEfficiencyByMode',
]);
const envelope = {
  sentinel: 'unchanged',
  ...row,
  trip: row,
  session: row,
  items: [row],
  total: 41,
  per_page: 7,
  page: 2,
  vehicles: [row],
  members: [row],
  invites: [row],
  users: [row],
  invitations: [row],
  memberships: [row],
  places: [row],
  metrics: [row],
  total_kwh: 20,
  by_type: { ac_kwh: 2, ac_l2_kwh: 3, dc_kwh: 15 },
  weekly: [{ week_start: '2026-01-01', kwh: 20, sessions: 2 }],
  daily: [{ day_local: '2026-01-01', day_start: row.ts, total_energy_kwh: '20', session_count: 2 }],
  daily_sessions: [{ ...row, day_start: row.ts }],
  access_token: 'synthetic-renewed',
  expires_in: 900,
  default_vehicle_id: null,
};

beforeEach(() => {
  setApiBaseUrl('https://synthetic.example.invalid');
  vi.useFakeTimers();
  vi.setSystemTime(new Date('2026-01-01T00:00:00Z'));
});
afterEach(() => {
  setApiBaseUrl(undefined);
  vi.restoreAllMocks();
  vi.useRealTimers();
});

describe('complete endpoint request/response characterization', () => {
  for (const [methodName, ...args] of cases) {
    const name = String(methodName);
    it(name, async () => {
      const client = new AuthenticatedTransport();
      client.setToken('synthetic-access');
      const payload = arrayMethods.has(name) ? [row] : envelope;
      const fetchMock = vi.spyOn(globalThis, 'fetch').mockImplementation(
        async () =>
          new Response(JSON.stringify(payload), {
            headers: { 'Content-Disposition': 'attachment; filename="synthetic.dump"' },
          })
      );
      const method = client[name as keyof AuthenticatedTransport] as (
        ...args: unknown[]
      ) => Promise<unknown>;
      const result = await method.apply(client, args);
      const output =
        typeof result === 'object' && result !== null && 'blob' in result
          ? {
              ...result,
              blob: { size: (result.blob as Blob).size, type: (result.blob as Blob).type },
            }
          : result;
      expect({
        requests: fetchMock.mock.calls.map(([url, init]) => ({
          url,
          ...init,
          ...(init?.signal ? { signal: 'AbortSignal' } : {}),
        })),
        result: isDeepStrictEqual(result, payload) ? 'unchanged synthetic response' : output,
      }).toMatchSnapshot();
    });
  }

  it('preserves the exported domain method sets', () => {
    expect(
      Object.fromEntries(
        Object.entries(clients)
          .filter(([name]) => name.endsWith('Client'))
          .map(([name, client]) => [name, Object.keys(client).sort()])
      )
    ).toMatchSnapshot();
  });

  it('preserves the complete public instance method surface', () => {
    const instance = new AuthenticatedTransport();
    const names = new Set<string>();
    for (
      let prototype = Object.getPrototypeOf(instance);
      prototype !== Object.prototype;
      prototype = Object.getPrototypeOf(prototype)
    ) {
      for (const name of Object.getOwnPropertyNames(prototype)) {
        if (name === 'constructor' || name.startsWith('_')) continue;
        if (
          cases.some(([method]) => method === name) ||
          [
            'apiFetch',
            'requestResponse',
            'proxyFetch',
            'authenticatedAsset',
            'uploadBackupArtifact',
            'setToken',
            'onAuthChange',
          ].includes(name)
        )
          names.add(name);
      }
    }
    expect([...names].sort()).toMatchSnapshot();
  });
});
