import React from 'react';
import { Link } from '@tanstack/react-router';
import { Check, Minus, TriangleAlert } from 'lucide-react';
import {
  useChargeGaps,
  useChargeHistoryBackfillStatus,
  useCurrentVehicleStatus,
  useDegradation,
  useLifetimeTrips,
  useRecoverMissedCharges,
  useResolvedVehicleSelection,
  useTripGaps,
  useVehicleHealth,
} from '@panda/hooks';
import type { Vehicle, VehicleHealth, VehicleStatus } from '@panda/types';
import { Skeleton } from '@panda/ui/primitives';
import { formatAppDate, formatAppDateTime, formatAppTime, startOfAppDay } from '@panda/ui/lib/dateTime';
import { formatMiles, formatNumber, formatPercent, getUnitPreferences } from '@panda/ui/lib/utils';
import { DEFAULT_TARGET_TIRE_PRESSURE_PSI } from '@panda/ui/lib/vehicleTires';
import { dedupeSoftwareHistory } from '../health/VehicleHealthPage';
import {
  batteryHealthPct,
  closures,
  missedCharges,
  sampleBars,
  softwareState,
  tireReadings,
  tripCoverage,
  vehicleHealthRows,
  type RowState,
  type StatusRow,
  type TireReading,
} from '../health/carModel';
import { vehicleConfiguration } from '../now/nowModel';
import { NoVehicleState } from '../../components/layout/NoVehicleState';
import { getFeedHealthIssue } from '../../components/layout/vehicleConnection';
import { emitToast } from '../../components/feedback/toast';
import { RAppLayout, RPageActions } from './RAppLayout';
import { RMissedDrives } from './RMissedDrives';
import { RSettingsEntry } from './RSettingsNavigation';
import { RVehicleTurn } from './RVehicleOrbit';
import { vehicleReading } from './readings';
import { vehicleLockReading } from './vehicleLockReading';
import './r-health.css';

const DAY_MS = 24 * 60 * 60 * 1000;

/** Car configuration under the title, for example "R2 Performance · Catalina Cove · 21" Tungsten wheels". */
function carSubtitle(vehicle: Vehicle) {
  const wheels = (vehicle as Vehicle & { wheel_option?: string | null }).wheel_option?.trim();
  // Rivian names the wheel with its tire, for example '21" Liquid Tungsten All-Season'. Keep the size and finish.
  const wheel = wheels?.replace(/\s+(all[- ]season|all[- ]terrain|performance|winter)\b.*$/i, '').replace(/\bLiquid\s+/i, '');
  return [vehicleConfiguration(vehicle), wheel ? `${wheel} wheels` : null].filter(Boolean).join(' · ');
}

/** A time today, or a short date and time on an earlier day. */
function clock(value: string | null | undefined) {
  if (!value) return null;
  const time = new Date(value).getTime();
  if (!Number.isFinite(time)) return null;
  if (time >= startOfAppDay(Date.now()).getTime()) return formatAppTime(value);
  // "Nov 7, 7:42 PM"; the year shows only for an earlier year.
  const sameYear = formatAppDate(value, { month: undefined, day: undefined }) === formatAppDate(Date.now(), { month: undefined, day: undefined });
  return formatAppDateTime(value, sameYear ? { year: undefined } : {});
}

function shortDate(value: string) {
  return formatAppDate(value, { year: undefined });
}

export function RVehicleHealth() {
  const { authReady, effectiveVehicleId, vehicleSelectionReady, vehicles } = useResolvedVehicleSelection();
  const vehicle = vehicles.find(item => item.id === effectiveVehicleId);
  const { data, isLoading } = useVehicleHealth(effectiveVehicleId);
  const ready = authReady && vehicleSelectionReady;

  return (
    <RAppLayout activeKey="car" footer={<RSettingsEntry />}>
      <div className="r-page-heading r-car-heading">
        <div className="r-car-heading-text">
          <h1>Car</h1>
          {vehicle ? <p>{carSubtitle(vehicle)}</p> : null}
        </div>
        <RPageActions />
      </div>
      {!ready ? (
        <CarSkeleton />
      ) : !effectiveVehicleId || !vehicle ? (
        <NoVehicleState title="No vehicle selected" description="Connect your Rivian account to see your car." />
      ) : isLoading && !data ? (
        <CarSkeleton />
      ) : (
        <CarContent vehicle={vehicle} health={data?.vehicle_id === undefined || data.vehicle_id === vehicle.id ? data : undefined} />
      )}
    </RAppLayout>
  );
}

function CarContent({ vehicle, health }: { vehicle: Vehicle; health: VehicleHealth | undefined }) {
  const { data: status } = useCurrentVehicleStatus(vehicle.id);
  const tripGaps = useTripGaps(vehicle.id);
  const degradation = useDegradation(vehicle.id);
  const canRecover = vehicle.membership_role === 'owner' || vehicle.membership_role === 'manager';
  const targetPsi = Math.round(vehicle.target_tire_pressure_psi ?? DEFAULT_TARGET_TIRE_PRESSURE_PSI);
  const tires = tireReadings(status, health?.tires, targetPsi);
  const software = softwareState(status, health);
  const odometer = vehicleReading(status, 'odometer_miles');
  const limit = vehicleReading(status, 'battery_limit');
  const batteryHealth = batteryHealthPct(degradation.data);
  const quiet = tripGaps.data?.parallax_quiet ?? null;
  const history = dedupeSoftwareHistory(health?.software_history ?? []);

  return (
    <div className="r-car">
      <RMissedDrives vehicleId={vehicle.id} canRecover={canRecover} />
      {quiet ? (
        <p className="r-health-notice" role="status">
          <TriangleAlert aria-hidden="true" />
          <span>
            Rivian’s live stream sent no data after {formatAppDateTime(quiet.last_frame_at)}, but the car moved at{' '}
            {formatAppTime(quiet.moved_at)}. R keeps reconnecting. Meanwhile, R records drives from the odometer.
          </span>
        </p>
      ) : null}

      <div className="r-car-grid">
        <div className="r-car-column">
          <div className="r-car-turn">
            <RVehicleTurn vehicle={vehicle} label="Turn the car. Drag it, or use the left and right arrow keys." />
          </div>
          <dl className="r-car-spec">
            <div><dt>Odometer</dt><dd>{odometer === null ? 'Not reported' : formatMiles(odometer)}</dd></div>
            <div><dt>Software</dt><dd>{software.current ?? 'Not reported'}</dd></div>
            <div><dt>Charge limit</dt><dd>{limit === null ? 'Not reported' : formatPercent(limit)}</dd></div>
            <div><dt>Battery health</dt><dd>{batteryHealth === null ? 'Not reported' : formatPercent(batteryHealth, 1)}</dd></div>
          </dl>

          <section className="r-car-section" aria-labelledby="r-car-tires">
            <h2 id="r-car-tires">Tires</h2>
            <TiresTopDown tires={tires} targetPsi={targetPsi} />
          </section>

          <section className="r-car-section" aria-labelledby="r-car-closures">
            <h2 id="r-car-closures">Closures</h2>
            <ul className="r-car-closures">
              {closures(status, health, vehicleLockReading(status)).map(item => (
                <li key={item.key} className="r-car-closure" data-state={item.state}>
                  <StateIcon state={item.state} />
                  <div><span>{item.label}</span><strong>{item.value}</strong></div>
                </li>
              ))}
            </ul>
          </section>
        </div>

        <div className="r-car-column">
          <section className="r-car-section" aria-labelledby="r-car-vehicle-health">
            <h2 id="r-car-vehicle-health">Vehicle health</h2>
            <StatusList rows={vehicleHealthRows(status, health, tires)} />
          </section>

          <section className="r-car-section" aria-labelledby="r-car-data-health">
            <h2 id="r-car-data-health">Data health</h2>
            <DataHealth vehicle={vehicle} status={status} health={health} canRecover={canRecover} odometer={odometer}
              tripGaps={tripGaps} />
          </section>

          <section className="r-car-section" aria-labelledby="r-car-software">
            <h2 id="r-car-software">Software history</h2>
            <div className="r-car-card r-car-pad">
              {history.length > 0 ? (
                <ol className="r-car-swline">
                  {history.map((entry, index) => {
                    const current = index === 0 && !entry.observed_until;
                    return (
                      <li key={`${entry.version}-${entry.installed_at}`} data-current={current || undefined}>
                        <strong>{entry.version}</strong>
                        <span>{current ? 'Current · ' : ''}{shortDate(entry.installed_at)}</span>
                      </li>
                    );
                  })}
                </ol>
              ) : (
                <p className="r-car-note">No software history recorded yet.</p>
              )}
            </div>
          </section>
        </div>
      </div>
    </div>
  );
}

function StateIcon({ state }: { state: RowState }) {
  const Icon = state === 'ok' ? Check : state === 'warn' ? TriangleAlert : Minus;
  return <span className="r-car-state-icon" data-state={state} aria-hidden="true"><Icon /></span>;
}

function StatusList({ rows }: { rows: StatusRow[] }) {
  return (
    <ul className="r-car-card r-car-hlist">
      {rows.map(row => <StatusItem key={row.key} row={row} />)}
    </ul>
  );
}

function StatusItem({ row }: { row: StatusRow }) {
  return (
    <li className="r-car-hrow" data-state={row.state} data-row={row.key}>
      <StateIcon state={row.state} />
      <div><span>{row.label}</span>{row.detail ? <small>{row.detail}</small> : null}</div>
      <strong>{row.value}</strong>
    </li>
  );
}

function pressureParts(psi: number | null) {
  if (psi === null) return { value: '–', unit: '' };
  if (getUnitPreferences().pressure_unit === 'kpa') return { value: formatNumber(psi * 6.894757293168361, 0), unit: 'kPa' };
  return { value: formatNumber(psi, 1), unit: 'psi' };
}

function TiresTopDown({ tires, targetPsi }: { tires: TireReading[]; targetPsi: number }) {
  const byPosition = Object.fromEntries(tires.map(tire => [tire.position, tire])) as Record<TireReading['position'], TireReading>;
  const tire = (position: TireReading['position']) => {
    const reading = byPosition[position];
    const { value, unit } = pressureParts(reading.psi);
    return (
      <div className="r-car-tire" data-tone={reading.tone}>
        <span>{reading.label}</span>
        <strong>{value}{unit ? <small> {unit}</small> : null}</strong>
        <span>{reading.note}</span>
      </div>
    );
  };
  const wheel = (tone: TireReading['tone']) => (tone === 'success' ? 'good' : tone === 'neutral' ? 'none' : 'warn');
  return (
    <div className="r-car-card r-car-pad">
      <div className="r-car-topdown">
        <div className="r-car-tcol">{tire('fl')}{tire('rl')}</div>
        <svg viewBox="0 0 120 220" aria-hidden="true" className="r-car-body">
          <rect className="r-car-body-shell" x="22" y="8" width="76" height="204" rx="30" />
          <rect className="r-car-body-glass" x="32" y="52" width="56" height="46" rx="10" />
          <rect className="r-car-body-glass" x="32" y="128" width="56" height="40" rx="10" />
          {([[10, 36, 'fl'], [98, 36, 'fr'], [10, 150, 'rl'], [98, 150, 'rr']] as const).map(([x, y, position]) => (
            <rect key={position} className="r-car-wheel" data-state={wheel(byPosition[position].tone)} x={x} y={y} width="12" height="34" rx="5" />
          ))}
        </svg>
        <div className="r-car-tcol">{tire('fr')}{tire('rr')}</div>
      </div>
      <p className="r-car-note">Target {targetPsi} psi</p>
    </div>
  );
}

function DataHealth({ vehicle, status, health, canRecover, odometer, tripGaps }: {
  vehicle: Vehicle;
  status: VehicleStatus | null | undefined;
  health: VehicleHealth | undefined;
  canRecover: boolean;
  odometer: number | null;
  tripGaps: ReturnType<typeof useTripGaps>;
}) {
  const chargeGaps = useChargeGaps(vehicle.id);
  const trips = useLifetimeTrips(vehicle.id);
  const coverage = tripCoverage(trips.data, odometer);
  const charges = missedCharges(chargeGaps.data, chargeGaps.isError);
  const bars = sampleBars(chargeGaps.data?.daily_samples);
  const rows: StatusRow[] = [
    connectionRow(vehicle, status, health),
    freshnessRow('location', 'Location', status?.location_ts, 'Last reading'),
    freshnessRow('battery', 'Battery level', status?.battery_level_ts, 'Last change'),
  ];

  if (coverage) {
    const moves = coverage.shortMoves === 1 ? '1 short move' : `${coverage.shortMoves} short moves`;
    rows.push({
      key: 'coverage', label: 'Odometer vs trips', state: coverage.coveredPct >= 98 ? 'ok' : 'warn',
      value: `${formatNumber(coverage.coveredPct, 1)}% covered`,
      detail: `${formatMiles(coverage.unrecordedMiles, 1)} in ${moves} not in a trip`,
    });
  } else {
    rows.push({ key: 'coverage', label: 'Odometer vs trips', state: 'none', value: trips.isPending ? 'Checking…' : 'Not reported' });
  }

  rows.push({
    key: 'missed-charges', label: 'Missed charges', state: charges.state, value: charges.value,
    detail: charges.state === 'ok'
      ? `Battery never rose while parked without a charge${charges.checked !== null ? ` (${charges.checked} parked periods checked)` : ''}`
      : charges.state === 'warn'
        ? 'The battery rose while parked, but no charge is recorded.'
        : chargeGaps.isError ? 'The check did not run. Try again later.' : undefined,
  });

  const gapCount = tripGaps.data?.gaps?.length ?? 0;
  rows.push(gapCount > 0
    ? { key: 'missed-drives', label: 'Missed drives', state: 'warn', value: `${gapCount} possible`, detail: 'Review them at the top of this page.' }
    : tripGaps.data
      ? { key: 'missed-drives', label: 'Missed drives', state: 'ok', value: 'None found',
        detail: coverage ? `Largest odometer move outside a trip: ${formatMiles(coverage.largestMoveMiles, 1)}` : undefined }
      : { key: 'missed-drives', label: 'Missed drives', state: 'none', value: tripGaps.isError ? 'Not checked' : 'Checking…' });

  return (
    <div className="r-car-card">
      <ul className="r-car-hlist">{rows.map(row => <StatusItem key={row.key} row={row} />)}</ul>
      <div className="r-car-pad r-car-samples">
        {bars.length > 0 ? (
          <>
            <div className="r-car-ctitle"><strong>Samples per day</strong><span>{bars.length} days</span></div>
            <div className="r-car-bars" role="img"
              aria-label={`Data samples per day for the last ${bars.length} days. ${bars.filter(bar => bar.low).length} days had few samples.`}>
              {bars.map(bar => (
                <span key={bar.day} data-low={bar.low || undefined} style={{ '--h': bar.height } as React.CSSProperties}
                  title={`${shortDate(`${bar.day}T12:00:00Z`)}: ${bar.samples.toLocaleString()} samples`} />
              ))}
            </div>
            <p className="r-car-note">
              Highlighted days had few samples. Trips or charges on those days can be incomplete.
              {coverage && coverage.activityMoves > 0
                ? ` ${coverage.activityMoves === 1 ? '1 gap' : `${coverage.activityMoves} gaps`} of 1 mi or more show in Activity.`
                : ''}
            </p>
          </>
        ) : null}
        <RecoverCharges vehicleId={vehicle.id} canRecover={canRecover} />
        <p className="r-car-note">
          Recover asks Rivian for your charging history and adds any charge R missed. Rivian keeps no trip history, so a
          missed drive shows only as odometer miles without a trip.
        </p>
      </div>
    </div>
  );
}

function connectionRow(vehicle: Vehicle, status: VehicleStatus | null | undefined, health: VehicleHealth | undefined): StatusRow {
  const base = { key: 'rivian', label: 'Rivian connection' };
  const feed = getFeedHealthIssue(status ?? health?.runtime ?? null);
  const lastRefresh = clock(status?.last_updated ?? health?.runtime?.last_event_at ?? null);
  if (feed?.key.startsWith('needs_reauth')) {
    return { ...base, state: 'warn', value: 'Reconnect needed', detail: 'The Rivian sign-in expired. Use Reconnect Rivian below.' };
  }
  if (feed) return { ...base, state: 'warn', value: 'Not receiving', detail: lastRefresh ? `Last data ${lastRefresh}` : undefined };
  if (!status && !health?.runtime) return { ...base, state: 'none', value: 'Not reported' };
  const since = vehicle.credential_issued_at ?? status?.credential_issued_at ?? vehicle.created_at;
  return {
    ...base, state: 'ok', value: 'Connected',
    detail: [since ? `Since ${shortDate(since)}` : null, lastRefresh ? `last refresh ${lastRefresh}` : null].filter(Boolean).join(' · ') || undefined,
  };
}

function freshnessRow(key: string, label: string, at: string | null | undefined, detail: string): StatusRow {
  const value = clock(at);
  if (!value || !at) return { key, label, state: 'none', value: 'Not reported' };
  return { key, label, state: Date.now() - new Date(at).getTime() > DAY_MS ? 'warn' : 'ok', value, detail };
}

function RecoverCharges({ vehicleId, canRecover }: { vehicleId: string; canRecover: boolean }) {
  const status = useChargeHistoryBackfillStatus(vehicleId);
  const recover = useRecoverMissedCharges(vehicleId);
  const started = React.useRef(false);
  const state = status.data?.status ?? null;
  const running = recover.isPending || state === 'running';
  React.useEffect(() => {
    if (!started.current || running) return;
    started.current = false;
    if (state === 'done') emitToast({ variant: 'success', title: 'Charging history recovered', message: 'Any missed charge is now in Activity.' });
    else if (state === 'error') emitToast({ variant: 'error', title: 'Could not recover charges', message: 'Try again in a minute.' });
  }, [running, state]);
  const run = () => {
    started.current = true;
    recover.mutate(undefined, {
      onError: () => {
        started.current = false;
        emitToast({ variant: 'error', title: 'Could not recover charges', message: 'Try again in a minute.' });
      },
    });
  };
  const message = running ? 'Recovering missed charges from Rivian…'
    : state === 'error' ? 'The last recovery failed. Try again.'
      : status.data?.history_backfilled_at ? `Last recovery ${formatAppDateTime(status.data.history_backfilled_at)}` : '';
  return (
    <>
      <div className="r-car-actions">
        {canRecover ? (
          <button type="button" className="r-recover" onClick={run} disabled={running} aria-busy={running || undefined}>
            {running ? 'Recovering…' : 'Recover missed charges'}
          </button>
        ) : null}
        <Link to="/settings" className="r-car-ghost">Reconnect Rivian</Link>
      </div>
      <p className="r-car-note" role="status">{canRecover ? message : 'An owner or manager can recover missed charges.'}</p>
    </>
  );
}

function CarSkeleton() {
  return (
    <div className="r-car" aria-busy="true">
      <div className="r-car-grid">
        <div className="r-car-column">
          <Skeleton className="h-64 rounded-2xl" />
          <Skeleton className="h-32 rounded-2xl" />
        </div>
        <div className="r-car-column">
          <Skeleton className="h-96 rounded-2xl" />
        </div>
      </div>
    </div>
  );
}
