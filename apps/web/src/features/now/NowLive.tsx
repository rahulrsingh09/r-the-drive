import { chargePlace } from '../../lib/placeNames';
import React from 'react';
import type { ChargeSession, Trip, Vehicle, VehicleStatus } from '@panda/types';
import type { LiveSession } from '@panda/hooks';
import { useAuth, useBasemapConfig, useChargeCurve, useLiveSession, useUserPreferences } from '@panda/hooks';
import { TripMapChart } from '@panda/ui/charts';
import { useDocumentTheme } from '@panda/ui/hooks';
import { formatAppTime } from '@panda/ui/lib/dateTime';
import { chargeKind } from '../charging/detail/chargeDetailModel';
import { CATEGORY_COLOR, LimitBar, NowHeader, NowStage } from './NowParts';
import { NowLineChart } from './NowCharts';
import { chargeCategory, durationText, n0, n1, shortPlace, vehicleName } from './nowModel';
import { useActiveTrip, useClock, useLiveSamples } from './useNowLive';
import type { Place } from '@panda/types';

const KM_PER_MILE = 1.609344;
const finite = (value: number | null | undefined): value is number => value != null && Number.isFinite(value);
const first = (...values: Array<number | null | undefined>) => values.find(finite) ?? null;

/** The live charge readings: the live-session read first, then the open session's live fields, then the vehicle status. */
export function liveChargeReadings(live: LiveSession | null | undefined, session: ChargeSession | undefined, status: VehicleStatus | null) {
  const rangeKm = first(live?.range_added_km, session?.live_range_added_km, session?.range_added_km);
  return {
    battery: first(live?.soc_pct, session?.live_soc_pct, status?.battery_level),
    powerKw: first(live?.power_kw, session?.live_power_kw),
    addedKwh: first(live?.energy_kwh, session?.live_total_charged_kwh, session?.energy_added_kwh),
    leftMin: first(live?.time_remaining_min, session?.live_time_remaining_min, status?.time_to_end_of_charge_min),
    rangeAddedMi: rangeKm == null ? null : rangeKm / KM_PER_MILE,
    startedAt: live?.started_at ?? session?.live_started_at ?? session?.started_at ?? null,
    at: live?.ts ?? status?.last_updated ?? null,
  };
}

/** Live charging: power, battery with a striped fill, energy added, time left, range added and the power curve so far. */
export function NowLiveCharging({ vehicle, status, session, place, places, control }: {
  vehicle: Vehicle; status: VehicleStatus | null; session: ChargeSession | undefined; place: string | null;
  places: readonly Place[] | undefined; control?: React.ReactNode;
}) {
  const live = useLiveSession(vehicle.id, true);
  const curve = useChargeCurve(session?.id ?? null, vehicle.id, true);
  const reading = liveChargeReadings(live.data, session, status);
  const limit = status?.battery_limit ?? null;
  const dc = session ? chargeKind(session) === 'dc' : /dc/i.test(live.data?.charger_type ?? '');
  const color = session ? CATEGORY_COLOR[chargeCategory(session, places)] : 'var(--now-other)';
  const name = session ? shortPlace(chargePlace(session, null).name) : place;
  const done = reading.battery != null && limit != null && reading.battery >= limit - 0.3;
  const estimated = (curve.data ?? []).some(point => point.power_method === 'soc_delta');
  const points = (curve.data ?? []).filter(point => finite(point.soc_pct) && finite(point.power_kw))
    .map(point => ({ x: point.soc_pct, y: point.power_kw })).sort((a, b) => a.x - b.x);
  const startSoc = first(session?.soc_start, points[0]?.x, reading.battery) ?? 0;
  const x0 = Math.floor(startSoc / 10) * 10;
  const x1 = Math.max(limit ?? 80, 80, Math.ceil((reading.battery ?? 0) / 10) * 10);
  const peak = Math.max(4, reading.powerKw ?? 0, ...points.map(point => point.y));
  const yMax = dc ? Math.max(250, Math.ceil(peak / 50) * 50) : Math.ceil(peak / 4) * 4;
  const marker = reading.battery != null && reading.powerKw != null ? { x: reading.battery, y: reading.powerKw } : null;
  const sub = [name, reading.startedAt ? `started ${formatAppTime(reading.startedAt)}` : null].filter(Boolean).join(' · ');
  return <div className="now" data-activity="charging">
    <NowHeader name={vehicleName(vehicle)} chipState="charging" chip={`Charging · ${dc ? 'DC fast' : 'AC'}`} sub={sub || undefined} control={control} />
    <div className="now-grid">
      <div className="now-main">
        <NowStage vehicle={vehicle} mode="charging"><div className="now-flow" aria-hidden="true"><i /><i /><i /></div></NowStage>
        <section className="now-card now-batt now-live-card" aria-label="Charging now">
          <div className="now-batt-row">
            <div><div className="now-label">Battery</div>
              <div className="now-big"><span className="now-num">{reading.battery == null ? '–' : n0(reading.battery)}</span><small>%</small></div></div>
            <div className="now-range"><div className="now-label">Charging at</div>
              <div className="now-range-v"><span className="now-num">{reading.powerKw == null ? '–' : reading.powerKw >= 20 ? n0(reading.powerKw) : n1(reading.powerKw)}</span> <small>kW</small></div></div>
          </div>
          {reading.battery != null && <LimitBar battery={reading.battery} limit={limit} animate={false} charging />}
          <div className="now-sum-grid now-live-grid">
            <div><div className="now-label">Added</div><div className="now-v now-num">{reading.addedKwh == null ? '–' : <>+{n1(Math.max(0, reading.addedKwh))} <small>kWh</small></>}</div></div>
            <div><div className="now-label">Time left</div><div className="now-v now-num">{done ? 'Done' : reading.leftMin != null && reading.leftMin > 0 ? durationText(reading.leftMin) : '–'}</div></div>
            <div><div className="now-label">Range added</div><div className="now-v now-num">{reading.rangeAddedMi == null ? '–' : <>+{n0(Math.max(0, reading.rangeAddedMi))} <small>mi</small></>}</div></div>
          </div>
        </section>
      </div>
      <div className="now-side">
        <h2 className="now-h2">This session</h2>
        <section className="now-card now-pad" aria-label="Power by battery level">
          <div className="now-ctitle"><b>Power by battery level</b><span className="now-muted">{reading.at ? formatAppTime(reading.at) : ''}</span></div>
          {points.length || marker ? <NowLineChart height={170} padTop={14} padBottom={22} label="Power by battery level so far" color={color}
            points={points} xDomain={[x0, x1]} yDomain={[0, yMax]} ticks={[0, yMax / 2, yMax]} tickFormat={value => `${n0(value)} kW`} marker={marker} />
            : <p className="now-note">The power curve appears after the first readings.</p>}
          <div className="now-axis-x" aria-hidden="true"><span>{x0}%</span><span>{x1}%</span></div>
          {estimated && <p className="now-note">Estimated: power comes from the battery level change where the charger reports none.</p>}
        </section>
      </div>
    </div>
  </div>;
}

/** Live driving: the map with the trip so far and a moving marker, trip miles, battery, range left and time. */
export function NowLiveDriving({ vehicle, status, lastTrip, control }: {
  vehicle: Vehicle; status: VehicleStatus | null; lastTrip: Trip | undefined; control?: React.ReactNode;
}) {
  const active = useActiveTrip(vehicle.id, true).data ?? null;
  const samples = useLiveSamples(status, true);
  const now = useClock(true);
  const { accessToken } = useAuth();
  const basemap = useBasemapConfig();
  const preferences = useUserPreferences();
  const dark = useDocumentTheme();

  const startOdometer = first(active?.start_odometer_mi, lastTrip?.end_odometer_mi);
  const odometer = status?.odometer_miles ?? null;
  const tripMiles = odometer != null && startOdometer != null && odometer >= startOdometer && odometer - startOdometer < 1000 ? odometer - startOdometer : null;
  const startedAt = active?.started_at ?? null;
  const minutes = startedAt ? Math.max(0, (now - Date.parse(startedAt)) / 60_000) : null;
  const battery = status?.battery_level ?? null;
  const socStart = first(active?.soc_start, lastTrip?.soc_end);
  // A start reading below the current one is not a trip start (the car charged since): show no value.
  const used = battery != null && socStart != null && socStart >= battery ? socStart - battery : null;
  const position = status?.latitude != null && status.longitude != null ? { lat: status.latitude, lng: status.longitude } : null;
  const trail = [
    ...(active?.points ?? []).map(point => ({ lat: point.lat, lng: point.lng })),
    ...samples.filter(sample => sample.lat != null && sample.lng != null).map(sample => ({ lat: sample.lat!, lng: sample.lng! })),
  ];
  const track = trail.length > 1 ? trail : position ? [position, position] : [];
  const began = startedAt ? Date.parse(startedAt) : samples[0]?.at ?? now;
  const batteryPoints = [
    ...(socStart != null && startedAt ? [{ x: 0, y: socStart }] : []),
    ...samples.filter(sample => sample.battery != null).map(sample => ({ x: Math.max(0, (sample.at - began) / 60_000), y: sample.battery! })),
  ];
  const span = Math.max(1, ...batteryPoints.map(point => point.x));
  const speed = status?.speed_mph;
  const from = lastTrip?.end_place ? `From ${shortPlace(lastTrip.end_place)}` : undefined;
  return <div className="now" data-activity="driving">
    <NowHeader name={vehicleName(vehicle)} chipState="driving" control={control} sub={from}
      chip={<>Driving{speed != null && speed > 0 ? <> · <span className="now-num">{n0(Math.min(speed, 120))} mph</span></> : null}</>} />
    <div className="now-grid">
      <div className="now-main">
        <div className="now-stage now-live-map">
          {track.length > 1 ? <TripMapChart track={track} activePoint={position} height={320} className="now-map"
            mapStyle={dark ? 'dark' : 'light'} mapStylePreference={preferences.data?.map_style ?? 'follow-theme'}
            accessToken={accessToken} basemapConfig={basemap.data}
            basemapError={basemap.isError ? 'Map configuration unavailable' : null} onBasemapRetry={() => void basemap.refetch()} />
            : <p className="now-map-empty">The map appears when the car reports its position.</p>}
        </div>
        <section className="now-card now-batt now-live-card" aria-label="This trip">
          <div className="now-batt-row">
            <div><div className="now-label">This trip</div>
              <div className="now-big"><span className="now-num">{tripMiles == null ? '–' : n1(tripMiles)}</span><small>mi</small></div></div>
            <div className="now-range"><div className="now-label">Battery</div>
              <div className="now-range-v"><span className="now-num">{battery == null ? '–' : n0(battery)}</span><small>%</small></div></div>
          </div>
          <div className="now-sum-grid now-live-grid">
            <div><div className="now-label">Time</div><div className="now-v now-num">{minutes == null ? '–' : durationText(minutes)}</div></div>
            <div><div className="now-label">Range left</div><div className="now-v now-num">{status?.range_miles == null ? '–' : <>{n0(status.range_miles)} <small>mi</small></>}</div></div>
            <div><div className="now-label">Used</div><div className="now-v now-num">{used == null ? '–' : <>{n0(used)}<small>%</small></>}</div></div>
          </div>
        </section>
      </div>
      <div className="now-side">
        <h2 className="now-h2">On the way</h2>
        <section className="now-card now-pad" aria-label="Battery on this trip">
          <div className="now-ctitle"><b>Battery</b><span className="now-muted">{status?.last_updated ? formatAppTime(status.last_updated) : ''}</span></div>
          {batteryPoints.length > 1 ? <NowLineChart height={140} padTop={14} padBottom={4} label="Battery so far" color="var(--now-acc)" area
            points={batteryPoints} xDomain={[0, span]} yDomain={[0, 100]} ticks={[0, 50, 100]} tickFormat={value => `${value}%`} />
            : <p className="now-note">The battery line fills in as the car reports new readings.</p>}
        </section>
      </div>
    </div>
  </div>;
}
