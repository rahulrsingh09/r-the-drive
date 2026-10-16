import type { Page } from '@playwright/test';
import { installRFixture } from './r-experience.fixture';

export async function installGraphFixture(page: Page, options: Parameters<typeof installRFixture>[1] = {}) {
  const fixture = await installRFixture(page, { populatedAdmin: true, populatedAnalytics: true, ...options });
  const points = Array.from({ length: 14 }, (_, i) => ({ ts: new Date(Date.UTC(2026, 9, 1, i * 12)).toISOString(), value: [72, 68, 60, 80, 76, 68, 65][i % 7] }));
  await page.route(/\/v1\/(battery\/(mileage|degradation)|metrics\/series|vehicles\/[^/]+\/idle-drain|trips\/tire-pressure-timeline)(\?|$)/, async route => {
    const url = new URL(route.request().url());
    let body: unknown = [];
    if (url.pathname.endsWith('/mileage')) body = points.map((p, i) => ({ ts: p.ts, odometer_mi: 12150 + i * 24, usable_kwh: 99.1 + Math.sin(i) * .2, range_mi: 218, projected_max_range_mi: 315 + Math.sin(i) * 6, degradation_pct: 1.2 }));
    else if (url.pathname.endsWith('/degradation')) body = points.map((p, i) => ({ ts: p.ts, usable_kwh: 99.1, rated_kwh: 100, capacity_pct: 99.1 + Math.sin(i) * .2 }));
    else if (url.pathname.endsWith('/series')) body = points;
    else if (url.pathname.endsWith('/idle-drain')) body = { vehicle_id: 'owner-vehicle', periods: points.filter((_, i) => i % 2 === 0).map((p, i) => ({ period_start: p.ts, period_end: new Date(Date.parse(p.ts) + 8 * 3600000).toISOString(), duration_hours: 8, sleep_share_pct: .9, state_coverage_pct: 1, soc_start: 72, soc_end: 71.4, soc_lost_pct: .6 + i * .04, drain_pct_per_hour: .075, range_start_mi: 218, range_end_mi: 216, range_lost_mi: 2, range_lost_per_hour_mi: .25, energy_drained_kwh: .6, avg_power_w: 75, has_reduced_range: false, validation_status: 'validated', validation_reason: null, sample_count: 12, start_sample_at: p.ts, end_sample_at: null, movement_detected: false, overlaps_trip: false, overlaps_charge: false })) };
    else body = { vehicle_id: 'owner-vehicle', from: points[0]!.ts, to: points.at(-1)!.ts, samples: points.map((p, i) => ({ ts: p.ts, tire_fl_psi: 44 + Math.sin(i), tire_fr_psi: 44.5 + Math.sin(i), tire_rl_psi: 43.5 + Math.sin(i), tire_rr_psi: 44 + Math.sin(i) })), trips: [] };
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });
  });
  return fixture;
}
