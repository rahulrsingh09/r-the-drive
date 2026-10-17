import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import * as color from '@panda/ui/lib/color';
import * as units from '@panda/ui/lib/utils';
import * as dates from '@panda/ui/lib/dateTime';
import * as status from '@panda/ui/lib/vehicleStatus';
import * as tires from '@panda/ui/lib/vehicleTires';
import { formatDriveMode, getDriveModeBadgeClass } from '@panda/ui/lib/driveMode';
import { resolveTripLocation } from '@panda/ui/lib/tripPresentation';
import { applyThemeRuntime, resetThemePreferences } from '@panda/ui/lib/theme';
import { AmbientOrbs, Badge, Button, Card, CardHeader, CardTitle, CardContent, ChartSkeleton, EmptyState, Input, PageLayout, StatCard, StatCardGrid, StatCardSkeleton, StatusBar, Switch, MetricTabs } from '@panda/ui/primitives';
import { chargingColumns, createTripColumns, phantomDrainColumns, DataTable, PhantomDrainPeriodCard, TableControls } from '@panda/ui/tables';
import type { VehicleStatus, Place } from '@panda/types';
import type { ChargeSessionRow, TripRow, PhantomDrainRow } from '@panda/ui/tables';

vi.mock('react-dom', async importOriginal => ({ ...await importOriginal<typeof import('react-dom')>(), createPortal: (children: React.ReactNode) => children }));

beforeEach(() => { localStorage.clear(); vi.useFakeTimers(); vi.setSystemTime(new Date('2026-10-17T12:34:56Z')); });
afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); resetThemePreferences(); });
const stable = (value: unknown) => JSON.parse(JSON.stringify(value, (_key, item) => typeof item === 'number' && Number.isFinite(item) ? Number(item.toFixed(8)) : item));

describe('foundation baseline contracts', () => {
  it('characterizes all color models, parsing and gamut boundaries', () => {
    const samples = ['#abc', ' #ABCDEF ', '#000', '#fff', '#12', 'transparent', 'rgb(300, 0, 30)', 'rgba(1 2 3 / 50%)', 'hsl(-30 70% 50%)', 'hsla(400, 0%, 100%, .4)'];
    const parsed = samples.map(value => [value, color.normalizeHex(value), color.parseCssColor(value), color.canonicalColor(value)]);
    const conversions = Array.from({ length: 80 }, (_, i) => {
      const rgb = { r: (i * 43) % 256, g: (i * 71) % 256, b: (i * 137) % 256 };
      return [rgb, color.rgbToHex(rgb), color.rgbToHsl(rgb), color.rgbToOklch(rgb), color.hslToRgb(color.rgbToHsl(rgb)), color.oklchToHex(color.rgbToOklch(rgb))];
    });
    const gamut = [0, 5, 50, 95, 100].flatMap(l => [0, .1, .4, 1].flatMap(c => [-30, 0, 90, 210, 400].map(h => [l, c, h, color.oklchToHex({ l, c, h })])));
    expect(stable({ parsed, conversions, gamut, contrast: samples.map(value => color.contrastRatio(value, '#ffffff')) })).toMatchSnapshot();
  });

  it('characterizes formatting, migration, precision and nonfinite input contracts', () => {
    const formatters = [units.formatNumber, units.formatSmartNumber, units.formatMiles, units.formatDistanceKm, units.formatKwh, units.formatMph, units.formatEfficiency, units.formatEfficiencyFromWhPerKm, units.formatMassKg, units.formatEfficiencyValue, units.formatEnergyPerDistance, units.formatPressure, units.formatAltitude, units.formatCurrency, units.formatDuration, units.formatTemp];
    const values = [null, undefined, NaN, -1000, -1, 0, .49, 1.5, 59.9, 60, 119.9, 1000, Infinity];
    const results = ['imperial', 'metric'].flatMap(mode => ['distance_per_energy', 'energy_per_distance'].map(efficiency => {
      units.setUnitSystem(mode as units.UnitSystem); units.setEfficiencyDisplay(efficiency as units.EfficiencyDisplay);
      return { mode, efficiency, prefs: units.getUnitPreferences(), label: units.getEfficiencyUnitLabel(), output: formatters.map(fn => values.map(value => fn(value))) };
    }));
    expect(results).toMatchSnapshot();
    localStorage.clear(); localStorage.setItem('rm-units', 'metric'); localStorage.setItem('rm-efficiency-display', 'energy_per_distance');
    expect(units.getUnitPreferences()).toMatchSnapshot('legacy migration');
    expect(values.map(value => [units.whPerMileToMiPerKwh(value), units.whPerMileToKmPerKwh(value), units.whPerMileToWhPerKm(value)])).toMatchSnapshot('finite conversions');
  });

  it('characterizes timezone projection, DST boundaries and date normalization', () => {
    const results = ['UTC', 'America/Los_Angeles', 'Asia/Kolkata'].map(timezone => {
      dates.setAppTimezone(timezone);
      return { timezone, offset: dates.formatTimezoneOffset(timezone), values: ['invalid', '2026-03-08T09:30:00Z', '2026-11-01T08:30:00Z', '2026-12-31T23:59:59.789Z'].map(value => {
        const parts = dates.getAppDateParts(value);
        return [parts, dates.startOfAppDay(value).toJSON(), dates.endOfAppDay(value).toJSON(), dates.shiftAppCalendarDays(value, 2).toJSON(), dates.formatAppDate(value), dates.formatAppTime(value), dates.formatAppDateTime(value), parts ? dates.appDatePartsToDate(parts).toJSON() : null];
      }) };
    });
    expect(results).toMatchSnapshot();
    expect(['2026-02-31', '2026-01-01', 'bad'].map(dates.formatAppCalendarDate)).toMatchSnapshot('calendar strings');
  });

  it('characterizes every status definition and tire thresholds', () => {
    const ids = ['hv_thermal', 'twelve_volt_health', 'ota_current_version', 'ota_available_version', 'charge_port_open', 'charger_derate_active', 'cabin_precon', 'defrost_active', 'pet_mode', 'seat_fl_heat', 'seat_fr_heat', 'seat_rl_heat', 'seat_rr_heat', 'seat_fl_vent', 'seat_fr_vent', 'steering_wheel_heat', 'tonneau_status', 'gear_guard_locked', 'brake_fluid_warning', 'wiper_fluid_warning', 'alarm_status', 'service_mode', 'window_status', 'unknown'];
    const values = [null, true, false, 'active', 'off', 'normal', 'critical_failure', 'unknown-value', 0, 3];
    const presentations = ids.map(id => [id, values.map(value => {
      const fields = status.getStatusDefinitionFields(id);
      const vehicle = Object.fromEntries(fields.map(field => [field, value]));
      try { return status.presentVehicleStatusDefinition(id, vehicle as unknown as VehicleStatus); } catch { return { rejectsInvalidFieldType: true }; }
    })]);
    const availability = ['current', 'historical', 'never_seen'].flatMap(availability => ['invalid_sensor', 'missing_recent_payload', null].map(reason_code => {
      const vehicle = { field_availability: { hv_thermal_event: { availability, reason_code, last_seen_at: '2026-01-01T12:00:00Z', latest_event_at: '2026-01-02T12:00:00Z', ever_seen: true } }, hv_thermal_event: 'normal' };
      return status.presentVehicleStatusDefinition('hv_thermal', vehicle as unknown as VehicleStatus);
    }));
    expect({ presentations, availability }).toMatchSnapshot();
    expect([null, 33.5, 34, 34.5, 35, 37, 37.5, 40, 45, NaN].flatMap(psi => ['', 'invalid_sensor', 'low'].map(value => [psi, value, tires.getTireHealthTone({ psi, status: value }), tires.formatTireLabel(psi, value)]))).toMatchSnapshot('tire thresholds');
    expect([null, 39.4, 40].map(tires.getTireHealthLegend)).toMatchSnapshot('tire legends');
  });

  it('characterizes drive aliases, place precedence and theme snapshots', () => {
    const modes = ['everyday', 'AllPurpose', 'All-Purpose', 'offRoadSportAuto', 'offroadrocks', 'soft sand', 'winter', 'unknown', 'custom-X'];
    expect(modes.map(value => [formatDriveMode(value), getDriveModeBadgeClass(value)])).toMatchSnapshot('drive');
    const places = [{ name: 'Local', latitude: 10, longitude: 10, radius_m: 500, address: { display_name: 'Local address' } }] as Place[];
    expect([{}, { start_lat: 0, start_lng: 0 }, { start_lat: 10, start_lng: 10 }, { start_address: 'Street', start_lat: 10, start_lng: 10 }, { start_place: 'Named', start_lat: 10, start_lng: 10 }, { start_lat: 11, start_lng: 11 }].map(trip => resolveTripLocation(trip, 'start', places))).toMatchSnapshot('places');
    expect(['classic', 'rad'].flatMap(palette => ['light', 'dark'].map(mode => applyThemeRuntime({ palette, mode } as Parameters<typeof applyThemeRuntime>[0])))).toMatchSnapshot('theme outputs');
  });

  it('freezes static primitive DOM and classes for individual template review', () => {
    const components = [<AmbientOrbs />, <Badge dot>Ready</Badge>, <Button loading iconRight="right">Save</Button>, <Card glass hoverable><CardHeader><CardTitle>Title</CardTitle></CardHeader><CardContent>Body</CardContent></Card>, <ChartSkeleton />, <EmptyState title="Empty" description="Details" action={{ label: 'Retry', onClick: vi.fn() }} />, <Input label="Some Label" hint="Hint" error="Error" iconLeft="L" iconRight="R" />, <PageLayout title="Page" titleAction="Action" titleActionPosition="left" titleActionAfter="After" subtitle="Subtitle">Body</PageLayout>, <StatCard label="Amount" value={2} delta={-2} deltaLabel="before" />, <StatCardGrid>Grid</StatCardGrid>, <StatCardSkeleton />, <Switch checked />, <MetricTabs active="a" tabs={[{ key: 'a', label: 'Alpha' }, { key: 'b', label: 'Beta' }]} onChange={vi.fn()}>Metric body</MetricTabs>, ...(['online', 'offline', 'connecting', 'unhealthy', 'error'] as const).map(onlineState => <StatusBar onlineState={onlineState} socPercent={34} rangeEstimateMi={110} />)];
    expect(components.map(component => renderToStaticMarkup(component))).toMatchSnapshot();
  });

  it('freezes table formatting and original row models', () => {
    const trip = { id: 'synthetic', started_at: '2026-01-01T12:00:00Z', ended_at: null, distance_mi: 15.7, duration_min: 30, efficiency_wh_mi: 350, soc_start: 80, soc_end: 74, drive_mode: 'sport', tags: [{ id: 'tag', name: '  local   trip ', color_token: 'neutral' }] } as TripRow;
    const charge = { id: 'synthetic', started_at: '2026-01-01T12:00:00Z', ended_at: null, duration_min: 30, energy_added_kwh: 10, soc_start: 30, soc_end: 40, peak_power_kw: 21, cost_usd: null, charger_type: null, network_vendor: 'Tesla', location_name: null, live_time_elapsed_seconds: 750, live_soc_pct: 43, live_power_kw: 30, live_total_charged_kwh: 12 } as ChargeSessionRow;
    const period = { period_start: '2026-01-01T12:00:00Z', period_end: null, duration_hours: 3.25, sleep_share_pct: .5, state_coverage_pct: .9, soc_start: 50, soc_end: 49, soc_lost_pct: 1, drain_pct_per_hour: .33, range_lost_mi: 2, range_lost_per_hour_mi: .5, energy_drained_kwh: 1, avg_power_w: 1100, has_reduced_range: true, validation_reason: 'low_coverage' } as PhantomDrainRow;
    expect([
      renderToStaticMarkup(<DataTable data={[trip]} columns={createTripColumns([], { onInfoClick: vi.fn() })} />),
      renderToStaticMarkup(<DataTable data={[charge, { ...charge, ended_at: '2026-01-01T12:30:00Z', network_vendor: null, charger_type: 'AC', cost_usd: 2.5 }]} columns={chargingColumns} />),
      renderToStaticMarkup(<DataTable data={[period]} columns={phantomDrainColumns} />),
      renderToStaticMarkup(<PhantomDrainPeriodCard period={period} />),
      renderToStaticMarkup(<TableControls search="" onSearchChange={vi.fn()} searchPlaceholder="Find" rowsPerPage={10} rowsPerPageOptions={[10, 25]} onRowsPerPageChange={vi.fn()} page={1} totalPages={0} totalItems={0} itemLabel="trip" />),
    ]).toMatchSnapshot();
  });
});
