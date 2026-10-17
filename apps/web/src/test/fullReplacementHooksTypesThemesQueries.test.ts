import { beforeEach, describe, expect, it, vi } from 'vitest';
import * as battery from '../../../../packages/hooks/src/useBattery';
import * as efficiency from '../../../../packages/hooks/src/useEfficiency';
import * as metrics from '../../../../packages/hooks/src/useMetrics';
import * as charging from '../../../../packages/hooks/src/useCharging';
import * as trips from '../../../../packages/hooks/src/useTrips';
import { useVehicleHealth } from '../../../../packages/hooks/src/useHealth';
import { useSummaryStats } from '../../../../packages/hooks/src/useStats';
import { useMe } from '../../../../packages/hooks/src/useMe';
import { useVehicles } from '../../../../packages/hooks/src/useVehicles';
import { useTelemetryLanes } from '../../../../packages/hooks/src/useTelemetryLanes';
import { useBasemapConfig } from '../../../../packages/hooks/src/useBasemapConfig';
import { useUserPreferences } from '../../../../packages/hooks/src/useUserPreferences';
import { useDashboardChartFavorites } from '../../../../packages/hooks/src/useDashboardChartFavorites';

const harness = vi.hoisted(() => ({ ready: true, calls: [] as unknown[], options: {} as Record<string, unknown> }));
vi.mock('../../../../packages/hooks/src/useAuthState', () => ({ useAuthReady: () => harness.ready }));
vi.mock('../../../../packages/hooks/src/useAuth', () => ({ useAuth: (selector: (state: object) => unknown) => selector({ accessToken: harness.ready ? 'token' : null, isAuthenticated: harness.ready, isBootstrapping: !harness.ready, userId: 'synthetic-user' }) }));
vi.mock('../../../../packages/hooks/src/api', () => ({ api: new Proxy({}, { get: (_, method) => (...args: unknown[]) => { harness.calls.push([method, args]); return Promise.resolve([]); } }) }));
vi.mock('@tanstack/react-query', async (original) => ({ ...await original<object>(), useQuery: (options: Record<string, unknown>) => { harness.options = options; return options; } }));

const cases: Array<[string, () => unknown]> = [
  ...Object.entries(battery).map(([name, hook]) => [name, () => (hook as (...args: unknown[]) => unknown)('v', null, null)] as [string, () => unknown]),
  ...Object.entries(efficiency).map(([name, hook]) => [name, () => hook('v', 'from', 'to', { tagIds: ['z', 'a', 'z'], untagged: true })] as [string, () => unknown]),
  ['useMetricCatalog', () => metrics.useMetricCatalog()],
  ['useMetricValue', () => metrics.useMetricValue('v', 'soc')],
  ['useMetricSeries', () => metrics.useMetricSeries('v', 'soc', '2026-01-01', '2026-01-02')],
  ['useMetricBatch', () => metrics.useMetricBatch('v', [{ metric: 'soc' }, { metric: 'speed', include_latest: false }], null, null, true, { untagged: true })],
  ['useChargeSessions', () => charging.useChargeSessions('v', null, null, 2, 10, ' needle ', '2026-01-01')],
  ['useChargeSession', () => charging.useChargeSession('c', 'v')],
  ['useSavedPlaces', () => charging.useSavedPlaces()],
  ['useChargingNetworkPreferences', () => charging.useChargingNetworkPreferences('v')],
  ['useChargeCurve', () => charging.useChargeCurve('c', 'v', true)],
  ['useChargeCurveAnalysis', () => charging.useChargeCurveAnalysis('v', null, null)],
  ['useChargingSummary', () => charging.useChargingSummary('v', null, null)],
  ['useChargingChartSeries', () => charging.useChargingChartSeries('v', null, null)],
  ['useChargingSchedule', () => charging.useChargingSchedule('v')],
  ['useDepartureSchedules', () => charging.useDepartureSchedules('v')],
  ['useLiveSession', () => charging.useLiveSession('v')],
  ['useTrips', () => trips.useTrips('v', null, null, 3, 7, ' term ', { tagIds: ['z', 'a', 'z'], tagMatch: 'any', untagged: true })],
  ['useTripMapRoutes', () => trips.useTripMapRoutes('v', null, null)],
  ['useTirePressureTimeline', () => trips.useTirePressureTimeline('v', null, null)],
  ['useTripTags', () => trips.useTripTags('v')],
  ...['useTrip', 'useTripDetailData', 'useTripTrack', 'useSpeedProfile', 'useElevationProfile', 'useTripPowerProfile', 'useTripDetailSeries'].map((name) => [name, () => (trips as unknown as Record<string, (id: string, vehicle: string) => unknown>)[name]!('t', 'v')] as [string, () => unknown]),
  ['useVehicleHealth', () => useVehicleHealth('v')], ['useSummaryStats', () => useSummaryStats('v')], ['useMe', useMe], ['useVehicles', useVehicles],
  ['useTelemetryLanes', () => useTelemetryLanes('v', { max_points: 17 })], ['useBasemapConfig', useBasemapConfig], ['useUserPreferences', useUserPreferences], ['useDashboardChartFavorites', useDashboardChartFavorites],
];

beforeEach(() => { harness.ready = true; harness.calls = []; });
describe('query contract characterization', () => {
  it('preserves keys, timing, placeholder policy and API argument order', async () => {
    const actual = [];
    for (const [name, hook] of cases) {
      hook();
      const options = harness.options;
      await (options.queryFn as () => Promise<unknown>)();
      const summary = Object.fromEntries(Object.entries(options).map(([key, value]) => [key, typeof value === 'function' ? '<function>' : value]));
      actual.push({ name, options: summary, call: harness.calls.pop() });
    }
    expect(actual).toMatchInlineSnapshot(`
      [
        {
          "call": [
            "getSoc",
            [
              "v",
              null,
              null,
              true,
            ],
          ],
          "name": "useSocHistory",
          "options": {
            "enabled": true,
            "placeholderData": "<function>",
            "queryFn": "<function>",
            "queryKey": [
              "battery",
              "soc",
              "v",
              null,
              null,
              true,
            ],
            "staleTime": 120000,
          },
        },
        {
          "call": [
            "getRange",
            [
              "v",
              null,
              null,
              true,
            ],
          ],
          "name": "useRangeHistory",
          "options": {
            "enabled": true,
            "placeholderData": "<function>",
            "queryFn": "<function>",
            "queryKey": [
              "battery",
              "range",
              "v",
              null,
              null,
              true,
            ],
            "staleTime": 120000,
          },
        },
        {
          "call": [
            "getPhantomDrain",
            [
              "v",
              null,
              null,
              true,
            ],
          ],
          "name": "usePhantomDrain",
          "options": {
            "enabled": true,
            "placeholderData": "<function>",
            "queryFn": "<function>",
            "queryKey": [
              "battery",
              "phantom",
              "v",
              null,
              null,
              true,
            ],
            "staleTime": 300000,
          },
        },
        {
          "call": [
            "getIdleDrainPeriods",
            [
              "v",
              null,
              null,
              250,
              6,
              true,
            ],
          ],
          "name": "usePhantomDrainPeriods",
          "options": {
            "enabled": true,
            "placeholderData": "<function>",
            "queryFn": "<function>",
            "queryKey": [
              "battery",
              "phantom-periods",
              "v",
              null,
              null,
              true,
              250,
              6,
            ],
            "staleTime": 300000,
          },
        },
        {
          "call": [
            "getParkedEnergy",
            [
              "v",
              null,
              null,
            ],
          ],
          "name": "useParkedEnergy",
          "options": {
            "enabled": true,
            "queryFn": "<function>",
            "queryKey": [
              "battery",
              "parked-energy",
              "v",
              null,
              null,
            ],
            "refetchInterval": 120000,
            "staleTime": 60000,
          },
        },
        {
          "call": [
            "getDegradation",
            [
              "v",
              null,
              null,
              true,
            ],
          ],
          "name": "useDegradation",
          "options": {
            "enabled": true,
            "placeholderData": "<function>",
            "queryFn": "<function>",
            "queryKey": [
              "battery",
              "degradation",
              "v",
              null,
              null,
              true,
            ],
            "staleTime": 3600000,
          },
        },
        {
          "call": [
            "getBatteryHealth",
            [
              "v",
            ],
          ],
          "name": "useBatteryHealth",
          "options": {
            "enabled": true,
            "placeholderData": "<function>",
            "queryFn": "<function>",
            "queryKey": [
              "battery",
              "health",
              "v",
            ],
            "staleTime": 3600000,
          },
        },
        {
          "call": [
            "getBatteryMileage",
            [
              "v",
              null,
              null,
              true,
            ],
          ],
          "name": "useBatteryMileage",
          "options": {
            "enabled": true,
            "placeholderData": "<function>",
            "queryFn": "<function>",
            "queryKey": [
              "battery",
              "mileage",
              "v",
              null,
              null,
              true,
            ],
            "staleTime": 3600000,
          },
        },
        {
          "call": [
            "getEfficiencySummary",
            [
              "v",
              "from",
              "to",
              false,
              {
                "tagIds": [
                  "a",
                  "z",
                ],
                "tagMatch": "all",
                "untagged": false,
              },
            ],
          ],
          "name": "useEfficiencySummary",
          "options": {
            "enabled": true,
            "placeholderData": "<function>",
            "queryFn": "<function>",
            "queryKey": [
              "efficiency",
              "summary",
              "v",
              "from",
              "to",
              false,
              {
                "tagIds": [
                  "a",
                  "z",
                ],
                "tagMatch": "all",
                "untagged": false,
              },
            ],
            "refetchOnMount": "always",
            "staleTime": 300000,
          },
        },
        {
          "call": [
            "getEfficiencyByMode",
            [
              "v",
              "from",
              "to",
              false,
              {
                "tagIds": [
                  "a",
                  "z",
                ],
                "tagMatch": "all",
                "untagged": false,
              },
            ],
          ],
          "name": "useEfficiencyByMode",
          "options": {
            "enabled": true,
            "placeholderData": "<function>",
            "queryFn": "<function>",
            "queryKey": [
              "efficiency",
              "by-mode",
              "v",
              "from",
              "to",
              false,
              {
                "tagIds": [
                  "a",
                  "z",
                ],
                "tagMatch": "all",
                "untagged": false,
              },
            ],
            "refetchOnMount": "always",
            "staleTime": 300000,
          },
        },
        {
          "call": [
            "getEfficiencyTrend",
            [
              "v",
              "from",
              "to",
              false,
              {
                "tagIds": [
                  "a",
                  "z",
                ],
                "tagMatch": "all",
                "untagged": false,
              },
            ],
          ],
          "name": "useEfficiencyTrend",
          "options": {
            "enabled": true,
            "placeholderData": "<function>",
            "queryFn": "<function>",
            "queryKey": [
              "efficiency",
              "trend",
              "v",
              "from",
              "to",
              false,
              {
                "tagIds": [
                  "a",
                  "z",
                ],
                "tagMatch": "all",
                "untagged": false,
              },
            ],
            "refetchOnMount": "always",
            "staleTime": 300000,
          },
        },
        {
          "call": [
            "getEfficiencyVsTemp",
            [
              "v",
              "from",
              "to",
              false,
              {
                "tagIds": [
                  "a",
                  "z",
                ],
                "tagMatch": "all",
                "untagged": false,
              },
            ],
          ],
          "name": "useEfficiencyVsTemp",
          "options": {
            "enabled": true,
            "placeholderData": "<function>",
            "queryFn": "<function>",
            "queryKey": [
              "efficiency",
              "vs-temp",
              "v",
              "from",
              "to",
              false,
              {
                "tagIds": [
                  "a",
                  "z",
                ],
                "tagMatch": "all",
                "untagged": false,
              },
            ],
            "refetchOnMount": "always",
            "staleTime": 600000,
          },
        },
        {
          "call": [
            "getEfficiencyByTag",
            [
              "v",
              "from",
              "to",
              false,
              {
                "tagIds": [
                  "a",
                  "z",
                ],
                "tagMatch": "all",
                "untagged": false,
              },
            ],
          ],
          "name": "useEfficiencyByTag",
          "options": {
            "enabled": true,
            "placeholderData": "<function>",
            "queryFn": "<function>",
            "queryKey": [
              "efficiency",
              "by-tag",
              "v",
              "from",
              "to",
              false,
              {
                "tagIds": [
                  "a",
                  "z",
                ],
                "tagMatch": "all",
                "untagged": false,
              },
            ],
            "refetchOnMount": "always",
            "staleTime": 300000,
          },
        },
        {
          "call": [
            "getMetricCatalog",
            [],
          ],
          "name": "useMetricCatalog",
          "options": {
            "enabled": true,
            "placeholderData": "<function>",
            "queryFn": "<function>",
            "queryKey": [
              "metrics",
              "catalog",
            ],
            "staleTime": 3600000,
          },
        },
        {
          "call": [
            "getMetricValue",
            [
              "v",
              "soc",
              null,
              null,
              true,
            ],
          ],
          "name": "useMetricValue",
          "options": {
            "enabled": true,
            "placeholderData": "<function>",
            "queryFn": "<function>",
            "queryKey": [
              "metrics",
              "value",
              "v",
              "soc",
              null,
              null,
              true,
            ],
            "staleTime": 120000,
          },
        },
        {
          "call": [
            "getMetricSeries",
            [
              "v",
              "soc",
              "2026-01-01",
              "2026-01-02",
              "15min",
              false,
            ],
          ],
          "name": "useMetricSeries",
          "options": {
            "enabled": true,
            "placeholderData": "<function>",
            "queryFn": "<function>",
            "queryKey": [
              "metrics",
              "series",
              "v",
              "soc",
              "2026-01-01",
              "2026-01-02",
              false,
              "15min",
            ],
            "staleTime": 120000,
          },
        },
        {
          "call": [
            "getMetricBatch",
            [
              {
                "bucket": "raw",
                "density": "full",
                "from": null,
                "lifetime": true,
                "metrics": [
                  {
                    "include_latest": true,
                    "include_series": true,
                    "metric": "soc",
                  },
                  {
                    "include_latest": false,
                    "include_series": true,
                    "metric": "speed",
                  },
                ],
                "to": null,
                "untagged": true,
                "vehicle_id": "v",
              },
            ],
          ],
          "name": "useMetricBatch",
          "options": {
            "enabled": true,
            "meta": {
              "persist": false,
            },
            "placeholderData": "<function>",
            "queryFn": "<function>",
            "queryKey": [
              "metrics",
              "batch",
              "v",
              null,
              null,
              true,
              "soc:1:1|speed:0:1",
              {
                "tagIds": [],
                "tagMatch": "all",
                "untagged": true,
              },
            ],
            "staleTime": 120000,
          },
        },
        {
          "call": [
            "listChargeSessions",
            [
              "v",
              null,
              null,
              2,
              10,
              "needle",
              true,
              "2026-01-01",
            ],
          ],
          "name": "useChargeSessions",
          "options": {
            "enabled": true,
            "placeholderData": "<function>",
            "queryFn": "<function>",
            "queryKey": [
              "charging",
              "list",
              "v",
              null,
              null,
              true,
              2,
              10,
              "needle",
              "2026-01-01",
            ],
            "refetchInterval": "<function>",
            "refetchOnMount": "always",
            "refetchOnReconnect": false,
            "refetchOnWindowFocus": false,
            "staleTime": 60000,
          },
        },
        {
          "call": [
            "getChargeSession",
            [
              "c",
              "v",
            ],
          ],
          "name": "useChargeSession",
          "options": {
            "enabled": true,
            "placeholderData": "<function>",
            "queryFn": "<function>",
            "queryKey": [
              "charging",
              "detail",
              "c",
              "v",
            ],
            "refetchInterval": "<function>",
            "refetchOnMount": "always",
            "refetchOnReconnect": false,
            "refetchOnWindowFocus": false,
            "staleTime": 300000,
          },
        },
        {
          "call": [
            "listPlaces",
            [],
          ],
          "name": "useSavedPlaces",
          "options": {
            "enabled": true,
            "queryFn": "<function>",
            "queryKey": [
              "places",
            ],
            "refetchOnMount": "always",
            "refetchOnReconnect": false,
            "refetchOnWindowFocus": false,
            "staleTime": 3600000,
          },
        },
        {
          "call": [
            "listChargingNetworkPreferences",
            [
              "v",
            ],
          ],
          "name": "useChargingNetworkPreferences",
          "options": {
            "enabled": true,
            "queryFn": "<function>",
            "queryKey": [
              "charging",
              "network-preferences",
              "v",
            ],
            "refetchOnMount": "always",
            "refetchOnReconnect": false,
            "refetchOnWindowFocus": false,
            "staleTime": 60000,
          },
        },
        {
          "call": [
            "getChargeCurve",
            [
              "c",
              "v",
            ],
          ],
          "name": "useChargeCurve",
          "options": {
            "enabled": true,
            "placeholderData": "<function>",
            "queryFn": "<function>",
            "queryKey": [
              "charging",
              "curve",
              "c",
              "v",
            ],
            "refetchInterval": 30000,
            "refetchOnMount": "always",
            "refetchOnReconnect": false,
            "refetchOnWindowFocus": false,
            "staleTime": 300000,
          },
        },
        {
          "call": [
            "getChargeCurveAnalysis",
            [
              "v",
              null,
              null,
              true,
            ],
          ],
          "name": "useChargeCurveAnalysis",
          "options": {
            "enabled": true,
            "placeholderData": "<function>",
            "queryFn": "<function>",
            "queryKey": [
              "charging",
              "curve-analysis",
              "v",
              null,
              null,
              true,
            ],
            "refetchOnMount": "always",
            "refetchOnReconnect": false,
            "refetchOnWindowFocus": false,
            "staleTime": 300000,
          },
        },
        {
          "call": [
            "getChargingSummary",
            [
              "v",
              null,
              null,
              true,
            ],
          ],
          "name": "useChargingSummary",
          "options": {
            "enabled": true,
            "placeholderData": "<function>",
            "queryFn": "<function>",
            "queryKey": [
              "charging",
              "summary",
              "v",
              null,
              null,
              true,
            ],
            "refetchOnMount": "always",
            "refetchOnReconnect": false,
            "refetchOnWindowFocus": false,
            "staleTime": 300000,
          },
        },
        {
          "call": [
            "getChargingChartSeries",
            [
              "v",
              null,
              null,
              true,
            ],
          ],
          "name": "useChargingChartSeries",
          "options": {
            "enabled": true,
            "placeholderData": "<function>",
            "queryFn": "<function>",
            "queryKey": [
              "charging",
              "chart-series",
              "v",
              null,
              null,
              true,
            ],
            "refetchOnMount": "always",
            "refetchOnReconnect": false,
            "refetchOnWindowFocus": false,
            "staleTime": 300000,
          },
        },
        {
          "call": [
            "getChargingSchedule",
            [
              "v",
            ],
          ],
          "name": "useChargingSchedule",
          "options": {
            "enabled": true,
            "queryFn": "<function>",
            "queryKey": [
              "schedules",
              "charging",
              "v",
            ],
            "staleTime": 300000,
          },
        },
        {
          "call": [
            "listDepartureSchedules",
            [
              "v",
            ],
          ],
          "name": "useDepartureSchedules",
          "options": {
            "enabled": true,
            "queryFn": "<function>",
            "queryKey": [
              "schedules",
              "departure",
              "v",
            ],
            "staleTime": 300000,
          },
        },
        {
          "call": [
            "getLiveSession",
            [
              "v",
            ],
          ],
          "name": "useLiveSession",
          "options": {
            "enabled": true,
            "queryFn": "<function>",
            "queryKey": [
              "live-session",
              "v",
            ],
            "refetchInterval": 30000,
            "staleTime": 0,
          },
        },
        {
          "call": [
            "listTrips",
            [
              "v",
              null,
              null,
              3,
              7,
              "term",
              true,
              {
                "tagIds": [
                  "a",
                  "z",
                ],
                "tagMatch": "any",
                "untagged": false,
              },
            ],
          ],
          "name": "useTrips",
          "options": {
            "enabled": true,
            "queryFn": "<function>",
            "queryKey": [
              "trips",
              "list",
              "v2",
              "v",
              null,
              null,
              true,
              3,
              7,
              "term",
              {
                "tagIds": [
                  "a",
                  "z",
                ],
                "tagMatch": "any",
                "untagged": false,
              },
            ],
            "refetchOnMount": "always",
            "refetchOnReconnect": false,
            "refetchOnWindowFocus": false,
            "staleTime": 60000,
          },
        },
        {
          "call": [
            "getTripMap",
            [
              "v",
              null,
              null,
              "",
              true,
              {
                "tagIds": [],
                "tagMatch": "all",
                "untagged": false,
              },
            ],
          ],
          "name": "useTripMapRoutes",
          "options": {
            "enabled": true,
            "gcTime": 600000,
            "meta": {
              "persist": false,
            },
            "queryFn": "<function>",
            "queryKey": [
              "trips",
              "map",
              "v1",
              "v",
              null,
              null,
              true,
              "",
              {
                "tagIds": [],
                "tagMatch": "all",
                "untagged": false,
              },
            ],
            "refetchOnMount": "always",
            "refetchOnReconnect": false,
            "refetchOnWindowFocus": false,
            "staleTime": 60000,
          },
        },
        {
          "call": [
            "getTirePressureTimeline",
            [
              "v",
              null,
              null,
              true,
              {
                "tagIds": [],
                "tagMatch": "all",
                "untagged": false,
              },
            ],
          ],
          "name": "useTirePressureTimeline",
          "options": {
            "enabled": true,
            "gcTime": 600000,
            "meta": {
              "persist": false,
            },
            "queryFn": "<function>",
            "queryKey": [
              "trips",
              "tire-pressure-timeline",
              "v1",
              "v",
              null,
              null,
              true,
              {
                "tagIds": [],
                "tagMatch": "all",
                "untagged": false,
              },
            ],
            "refetchOnMount": "always",
            "refetchOnReconnect": false,
            "refetchOnWindowFocus": false,
            "staleTime": 60000,
          },
        },
        {
          "call": [
            "listTripTags",
            [
              "v",
            ],
          ],
          "name": "useTripTags",
          "options": {
            "enabled": true,
            "queryFn": "<function>",
            "queryKey": [
              "trip-tags",
              "v",
            ],
            "refetchOnWindowFocus": false,
            "staleTime": 300000,
          },
        },
        {
          "call": [
            "getTrip",
            [
              "t",
              "v",
            ],
          ],
          "name": "useTrip",
          "options": {
            "enabled": true,
            "placeholderData": "<function>",
            "queryFn": "<function>",
            "queryKey": [
              "trips",
              "detail",
              "t",
              "v",
            ],
            "refetchOnMount": "always",
            "refetchOnReconnect": false,
            "refetchOnWindowFocus": false,
            "staleTime": 300000,
          },
        },
        {
          "call": [
            "getTripDetailData",
            [
              "t",
              "v",
            ],
          ],
          "name": "useTripDetailData",
          "options": {
            "enabled": true,
            "gcTime": 1800000,
            "meta": {
              "persist": false,
            },
            "placeholderData": "<function>",
            "queryFn": "<function>",
            "queryKey": [
              "trips",
              "detail-data",
              "v1",
              "t",
              "v",
            ],
            "refetchOnMount": "always",
            "refetchOnReconnect": false,
            "refetchOnWindowFocus": false,
            "staleTime": Infinity,
          },
        },
        {
          "call": [
            "getTripTrack",
            [
              "t",
              "v",
            ],
          ],
          "name": "useTripTrack",
          "options": {
            "enabled": true,
            "meta": {
              "persist": false,
            },
            "placeholderData": "<function>",
            "queryFn": "<function>",
            "queryKey": [
              "trips",
              "track",
              "t",
              "v",
            ],
            "refetchOnMount": "always",
            "refetchOnReconnect": false,
            "refetchOnWindowFocus": false,
            "staleTime": 300000,
          },
        },
        {
          "call": [
            "getSpeedProfile",
            [
              "t",
              "v",
            ],
          ],
          "name": "useSpeedProfile",
          "options": {
            "enabled": true,
            "meta": {
              "persist": false,
            },
            "placeholderData": "<function>",
            "queryFn": "<function>",
            "queryKey": [
              "trips",
              "speed",
              "t",
              "v",
            ],
            "refetchOnMount": "always",
            "refetchOnReconnect": false,
            "refetchOnWindowFocus": false,
            "staleTime": 300000,
          },
        },
        {
          "call": [
            "getElevationProfile",
            [
              "t",
              "v",
            ],
          ],
          "name": "useElevationProfile",
          "options": {
            "enabled": true,
            "meta": {
              "persist": false,
            },
            "placeholderData": "<function>",
            "queryFn": "<function>",
            "queryKey": [
              "trips",
              "elevation",
              "t",
              "v",
            ],
            "refetchOnMount": "always",
            "refetchOnReconnect": false,
            "refetchOnWindowFocus": false,
            "staleTime": 300000,
          },
        },
        {
          "call": [
            "getTripPowerProfile",
            [
              "t",
              "v",
            ],
          ],
          "name": "useTripPowerProfile",
          "options": {
            "enabled": true,
            "meta": {
              "persist": false,
            },
            "placeholderData": "<function>",
            "queryFn": "<function>",
            "queryKey": [
              "trips",
              "power",
              "t",
              "v",
            ],
            "refetchOnMount": "always",
            "refetchOnReconnect": false,
            "refetchOnWindowFocus": false,
            "staleTime": 300000,
          },
        },
        {
          "call": [
            "getTripDetailSeries",
            [
              "t",
              "v",
            ],
          ],
          "name": "useTripDetailSeries",
          "options": {
            "enabled": true,
            "meta": {
              "persist": false,
            },
            "placeholderData": "<function>",
            "queryFn": "<function>",
            "queryKey": [
              "trips",
              "series",
              "t",
              "v",
            ],
            "refetchOnMount": "always",
            "refetchOnReconnect": false,
            "refetchOnWindowFocus": false,
            "staleTime": 300000,
          },
        },
        {
          "call": [
            "getVehicleHealth",
            [
              "v",
            ],
          ],
          "name": "useVehicleHealth",
          "options": {
            "enabled": true,
            "placeholderData": "<function>",
            "queryFn": "<function>",
            "queryKey": [
              "vehicles",
              "health",
              "v",
            ],
            "refetchOnMount": "always",
            "staleTime": 60000,
          },
        },
        {
          "call": [
            "getStats",
            [
              "v",
            ],
          ],
          "name": "useSummaryStats",
          "options": {
            "enabled": true,
            "placeholderData": "<function>",
            "queryFn": "<function>",
            "queryKey": [
              "stats",
              "v",
            ],
            "staleTime": 600000,
          },
        },
        {
          "call": [
            "me",
            [],
          ],
          "name": "useMe",
          "options": {
            "enabled": true,
            "placeholderData": "<function>",
            "queryFn": "<function>",
            "queryKey": [
              "me",
            ],
            "refetchOnReconnect": false,
            "refetchOnWindowFocus": false,
            "staleTime": 3600000,
          },
        },
        {
          "call": [
            "listVehicles",
            [],
          ],
          "name": "useVehicles",
          "options": {
            "enabled": true,
            "placeholderData": "<function>",
            "queryFn": "<function>",
            "queryKey": [
              "vehicles",
            ],
            "staleTime": 300000,
          },
        },
        {
          "call": [
            "getTelemetryLanes",
            [
              "v",
              {
                "max_points": 17,
              },
            ],
          ],
          "name": "useTelemetryLanes",
          "options": {
            "enabled": true,
            "placeholderData": "<function>",
            "queryFn": "<function>",
            "queryKey": [
              "vehicles",
              "telemetry-lanes",
              "v",
              {
                "max_points": 17,
              },
            ],
            "staleTime": 120000,
          },
        },
        {
          "call": [
            "apiFetch",
            [
              "GET",
              "/v1/external/basemap/config",
            ],
          ],
          "name": "useBasemapConfig",
          "options": {
            "enabled": true,
            "queryFn": "<function>",
            "queryKey": [
              "external",
              "basemap",
              "config",
              "v2",
            ],
            "refetchOnWindowFocus": false,
            "retry": 1,
            "staleTime": 300000,
          },
        },
        {
          "call": [
            "apiFetch",
            [
              "GET",
              "/v1/auth/preferences",
            ],
          ],
          "name": "useUserPreferences",
          "options": {
            "enabled": true,
            "queryFn": "<function>",
            "queryKey": [
              "unit-preferences",
            ],
            "staleTime": 300000,
          },
        },
        {
          "call": [
            "getDashboardChartFavorites",
            [],
          ],
          "name": "useDashboardChartFavorites",
          "options": {
            "enabled": true,
            "queryFn": "<function>",
            "queryKey": [
              "auth",
              "dashboard-chart-favorites",
              "synthetic-user",
            ],
            "refetchOnReconnect": false,
            "refetchOnWindowFocus": false,
            "staleTime": 300000,
          },
        },
      ]
    `);
  });
  it('disables every characterized query before bootstrap completes', () => {
    harness.ready = false;
    for (const [, hook] of cases) { hook(); expect(harness.options.enabled).toBe(false); }
    expect(harness.calls).toEqual([]);
  });
  it('polls only unfinished charging sessions', () => {
    charging.useChargeSessions('v', null, null);
    const list = harness.options.refetchInterval as (q: unknown) => number | false;
    expect([list({ state: {} }), list({ state: { data: { items: [{ ended_at: 'done' }] } } }), list({ state: { data: { items: [{ ended_at: null }] } } })]).toEqual([false, false, 30000]);
    charging.useChargeSession('c', 'v');
    const detail = harness.options.refetchInterval as (q: unknown) => number | false;
    expect([detail({ state: {} }), detail({ state: { data: { ended_at: null } } }), detail({ state: { data: { ended_at: 'done' } } })]).toEqual([false, 30000, false]);
  });
});
