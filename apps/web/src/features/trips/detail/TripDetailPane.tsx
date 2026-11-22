import React from 'react';
import { Check, ChevronRight, TriangleAlert } from 'lucide-react';
import { useSavedPlaces, useTripDetailData } from '@panda/hooks';
import { Button, Skeleton } from '@panda/ui/primitives';
import { formatAppDate, formatAppTime } from '@panda/ui/lib/dateTime';
import { formatDriveMode } from '@panda/ui/lib/driveMode';
import { getUnitPreferences } from '@panda/ui/lib/utils';
import { useEfficiencyUnit } from '../../r-experience/efficiencyUnit';
import { duration, n0, n1, shortPlace, SHORT_TRIP_MI, toActivityTrip, tripTitle, type RouteCoordinate } from '../activity/activityModel';
import { LineChart } from '../activity/ActivityCharts';
import { buildReplayModel, replaySamples } from '../activity/replayModel';
import { buildTimeline, regenEnergyKwh, summarizeElevation, temperatureText } from './tripDetailModel';
import { createReplayStore, useReplayState, type ReplayStore } from './replayStore';
import { TripReplay } from './TripReplay';

/** Odometer readings closer than this to the trip distance match it. */
const ODOMETER_MATCH_MI = 0.3;
const METERS_TO_FEET = 3.28084;

export interface TripDetailPaneProps {
  tripId: string;
  vehicleId: string;
  /** Route preview from the Activity list, when it has one. */
  routePreview?: RouteCoordinate[] | null;
  header: React.ReactNode;
}

/** Trip detail (v5): replay map, hero, odometer check, battery chart and details. */
export function TripDetailPane({ tripId, vehicleId, routePreview = null, header }: TripDetailPaneProps) {
  const detail = useTripDetailData(tripId, vehicleId);
  const places = useSavedPlaces();
  const efficiency = useEfficiencyUnit();
  const detailData = detail.data?.trip?.id === tripId ? detail.data : undefined;
  const trip = React.useMemo(
    () => (detailData ? toActivityTrip(detailData.trip, places.data ?? [], routePreview) : null),
    [detailData, places.data, routePreview],
  );
  const samples = React.useMemo(() => replaySamples(detailData?.samples), [detailData?.samples]);
  const model = React.useMemo(
    () => (trip ? buildReplayModel(trip.route, samples, trip.minutes, trip.miles) : null),
    [samples, trip],
  );
  const totalMinutes = model?.totalMinutes ?? 0;
  // The replay clock lives as long as the trip and its length; refreshed data does not reset it.
  const store = React.useMemo(() => createReplayStore(totalMinutes), [tripId, totalMinutes]); // eslint-disable-line react-hooks/exhaustive-deps
  const timeline = React.useMemo(() => buildTimeline(detailData?.trip.started_at, detailData?.samples), [detailData]);

  if (!trip || !model) {
    return (
      <div className="r-act-detail" data-trip-detail="loading">
        {header}
        {detail.isError ? (
          <div className="r-act-card pad">
            <p>The trip did not load.</p>
            <Button type="button" variant="secondary" className="mt-3 min-h-11" onClick={() => void detail.refetch()}>Try again</Button>
          </div>
        ) : (
          <div className="r-act-dbody" aria-busy="true"><Skeleton className="h-72 w-full" /><Skeleton className="mt-4 h-10 w-2/3" /><Skeleton className="mt-4 h-36 w-full" /></div>
        )}
      </div>
    );
  }

  const prefs = getUnitPreferences();
  const used = trip.socStart != null && trip.socEnd != null ? trip.socStart - trip.socEnd : null;
  const odometerMiles = trip.odometerStart != null && trip.odometerEnd != null ? trip.odometerEnd - trip.odometerStart : null;
  const odometerDiff = odometerMiles != null ? odometerMiles - trip.miles : null;
  const matches = odometerDiff != null && Math.abs(odometerDiff) <= ODOMETER_MATCH_MI;
  const regen = regenEnergyKwh(timeline);
  const elevation = summarizeElevation(timeline);
  const averageMph = trip.minutes > 0 ? trip.miles / (trip.minutes / 60) : null;
  const topMph = detailData!.trip.max_speed_mph;
  const outside = detailData!.trip.outside_temp_c;
  const mode = detailData!.trip.drive_mode;
  const approximate = [...new Set([trip.from, trip.to].filter((place) => place.approximate).map((place) => shortPlace(place.name)))];
  const measuredEfficiency = trip.whPerMile != null && trip.miles >= SHORT_TRIP_MI ? efficiency.format(trip.whPerMile) : null;

  const details: Array<[string, string]> = [
    ['Energy used', trip.trip.energy_used_kwh != null ? `${n1(trip.trip.energy_used_kwh)} kWh` : '–'],
    ['Regenerated', regen != null ? `${n1(regen)} kWh` : '–'],
    ['Average speed', averageMph != null ? `${n0(averageMph)} mph` : '–'],
    // A top speed below the average is a bad reading; it stays hidden.
    ['Top speed', topMph != null && averageMph != null && topMph >= averageMph ? `${n0(topMph)} mph` : '–'],
    ['Climb / descent', elevation ? `${n0(elevation.gainM * METERS_TO_FEET)} / ${n0(elevation.lossM * METERS_TO_FEET)} ft` : '–'],
    ['Outside', outside != null ? temperatureText(outside, prefs) : '–'],
    ['Drive mode', mode ? formatDriveMode(mode) : '–'],
  ];

  return (
    <div className="r-act-detail" data-trip-detail={trip.id}>
      {header}
      <div className="r-act-dbody">
        {model.route.length > 1
          ? <TripReplay model={model} store={store} startedAt={trip.start} />
          : <div className="r-act-map r-act-map-empty"><p>No route was recorded for this trip.</p></div>}
        <h1 className="r-act-dtitle">{tripTitle(trip)}</h1>
        <p className="muted">{formatAppDate(trip.start, { weekday: 'short', month: 'short', day: 'numeric', year: undefined })} · {formatAppTime(trip.start)} – {formatAppTime(trip.end)}</p>
        <div className="r-act-hero" data-trip-hero>
          <div><p className="r-act-label">Trip</p><p className="v num" data-testid="trip-distance">{n1(trip.miles)} <small>mi</small></p></div>
          <div><p className="r-act-label">Duration</p><p className="v num">{duration(trip.minutes)}</p></div>
          <div>
            <p className="r-act-label">Efficiency</p>
            <p className="v num" data-testid="trip-efficiency">{measuredEfficiency ? <>{measuredEfficiency} <small>{efficiency.label}</small></> : <small>Too short to measure</small>}</p>
          </div>
          <div><p className="r-act-label">Battery used</p><p className="v num">{used != null ? <>{n0(used)}<small>%</small></> : '–'}</p></div>
        </div>

        <h2 className="r-act-h2">Odometer check</h2>
        <section className="r-act-card pad" data-odometer-check>
          <div className="r-act-odo">
            <div><p className="r-act-label">Start</p><p className="v num">{trip.odometerStart != null ? n1(trip.odometerStart) : '–'}</p></div>
            <ChevronRight className="r-act-odo-arrow" aria-label="to" />
            <div><p className="r-act-label">End</p><p className="v num">{trip.odometerEnd != null ? n1(trip.odometerEnd) : '–'}</p></div>
            <p className={`r-act-odo-res ${matches ? 'ok' : 'warn'}`} data-odometer-result>
              {odometerDiff == null ? 'No odometer reading'
                : matches ? <><Check aria-hidden="true" />Matches the trip</>
                  : <><TriangleAlert aria-hidden="true" />Differs by {n1(Math.abs(odometerDiff))} mi</>}
            </p>
          </div>
          <p className="r-act-note">Odometer miles, start to end of this trip. The trip distance comes from the route.</p>
        </section>

        <h2 className="r-act-h2">Battery</h2>
        <section className="r-act-card pad">
          <TripBatteryChart store={store} total={model.totalMinutes} battery={model.battery} start={trip.start} socStart={trip.socStart} socEnd={trip.socEnd} />
        </section>

        <h2 className="r-act-h2">Details</h2>
        <dl className="r-act-card r-act-kv" data-trip-details>
          {details.map(([label, value]) => <div key={label}><dt className="r-act-label">{label}</dt><dd className="v num">{value}</dd></div>)}
        </dl>
        {approximate.length ? <p className="r-act-note">{approximate.join(' and ')} is the nearest town, not an address.</p> : null}
      </div>
    </div>
  );
}

/** Battery level during the trip. The cursor follows the replay. */
function TripBatteryChart({ store, total, battery, start, socStart, socEnd }: {
  store: ReplayStore;
  total: number;
  battery: Array<[number, number]>;
  start: Date;
  socStart: number | null;
  socEnd: number | null;
}) {
  const state = useReplayState(store);
  const time = (minute: number) => formatAppTime(start.getTime() + minute * 60_000);
  if (battery.length < 2) return <p className="r-act-empty">No battery samples for this trip.</p>;
  const first = battery[0]!;
  const last = battery.at(-1)!;
  const showCursor = state.playing || state.minute < total;
  return (
    <LineChart
      label={`Battery level during the trip, ${n0(socStart ?? first[1])}% to ${n0(socEnd ?? last[1])}%`}
      points={battery.map(([minute, value]) => ({ x: minute, y: value, tip: `${n0(value)}% · ${time(minute)}` }))}
      yDomain={[0, 100]}
      ticks={[0, 50, 100]}
      tickFormat={(value) => `${value}%`}
      color="var(--r-act-accent)"
      area
      xLabels={[[first[0], time(first[0]), 'start'], [last[0], time(last[0]), 'end']]}
      endLabels={[`${n0(first[1])}%`, `${n0(last[1])}%`]}
      cursorX={showCursor ? state.minute : null}
      testId="trip-battery-chart"
    />
  );
}
