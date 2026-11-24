import React from 'react';
import { useChargeCurve, useChargeSession, useResolvedVehicleSelection, useSavedPlaces, useUpdateChargeSession } from '@panda/hooks';
import type { ChargeSession, Place } from '@panda/types';
import { Button, Skeleton } from '@panda/ui/primitives';
import { formatAppDate, formatAppTime } from '@panda/ui/lib/dateTime';
import {
  CATEGORY_LABEL, chargeTitle, duration, n0, n1, shortPlace, toActivityCharge, usd, type ActivityCharge, type PlaceCategory,
} from '../../trips/activity/activityModel';
import { ExpandableChart, LineChart } from '../../trips/activity/ActivityCharts';
import { Segmented } from '../../trips/activity/Segmented';
import { costEntry, type CostEntryMode } from './chargeDetailModel';
import { usePlaceTag } from './usePlaceTag';

export const CATEGORY_COLOR: Record<PlaceCategory, string> = {
  home: 'var(--r-act-home)', office: 'var(--r-act-office)', public: 'var(--r-act-public)',
};

export interface ChargeDetailPaneProps {
  sessionId: string;
  vehicleId: string;
  canManage: boolean;
  /** Loaded charges, for the count at this place and for the tag. */
  charges: ActivityCharge[];
  header: React.ReactNode;
}

function homeRate(places: Place[]) {
  const home = places.find((place) => place.is_home)?.charging;
  return home && home.billing_type !== 'tou' ? (home.billing_type === 'free' ? 0 : home.rate) : null;
}

function officeRate(places: Place[]) {
  const office = places.find((place) => place.is_work)?.charging;
  return office ? (office.billing_type === 'free' ? 0 : office.rate) : 0;
}

/** Why the charge costs what it costs. */
export function costReason(charge: ActivityCharge, places: Place[]) {
  const method = charge.session.cost_method;
  if (method === 'user_manual') return 'You entered this cost.';
  if (method === 'user_free') return 'You marked this charge as free.';
  if (method === 'network_free' || method === 'rivian_free_session') return 'This charge was free.';
  if (method === 'rivian_paid_total') return 'Rivian reported what you paid.';
  if (charge.category === 'home') {
    const rate = homeRate(places);
    return rate != null ? `Home rate $${rate.toFixed(4)} per kWh × ${n1(charge.kwh)} kWh.` : 'Home uses the time-of-use rates in Settings.';
  }
  if (charge.category === 'office') {
    const rate = officeRate(places);
    return rate ? `Office rate $${rate.toFixed(4)} per kWh.` : 'Office charging is free.';
  }
  return 'This place has no rate. Enter what you paid.';
}

/** Charge detail (v5): hero, place tag, cost, charge level and estimated power. */
export function ChargeDetailPane({ sessionId, vehicleId, canManage, charges, header }: ChargeDetailPaneProps) {
  const sessionQuery = useChargeSession(sessionId, vehicleId);
  const placesQuery = useSavedPlaces();
  const places = React.useMemo(() => placesQuery.data ?? [], [placesQuery.data]);
  const session = sessionQuery.data?.id === sessionId ? sessionQuery.data : undefined;
  const listed = charges.find((charge) => charge.id === sessionId) ?? null;
  const charge = React.useMemo(() => (session ? toActivityCharge(session, places) : listed), [listed, places, session]);
  const curveQuery = useChargeCurve(sessionId, vehicleId, charge?.active ?? false);
  const { vehicles } = useResolvedVehicleSelection();
  const capacity = vehicles.find((vehicle) => vehicle.id === vehicleId)?.battery_capacity_kwh ?? null;

  if (!charge) {
    return (
      <div className="r-act-detail" data-charge-detail="loading">
        {header}
        {sessionQuery.isError ? (
          <div className="r-act-card pad">
            <p>The charge did not load.</p>
            <Button type="button" variant="secondary" className="mt-3 min-h-11" onClick={() => void sessionQuery.refetch()}>Try again</Button>
          </div>
        ) : (
          <div className="r-act-dbody" aria-busy="true"><Skeleton className="h-10 w-2/3" /><Skeleton className="mt-4 h-36 w-full" /><Skeleton className="mt-4 h-48 w-full" /></div>
        )}
      </div>
    );
  }

  const curve = (curveQuery.data ?? []).filter((point) => point.minutes_elapsed != null);
  const estimated = curve.length > 0 && curve.every((point) => point.power_method !== 'recorded');
  const minutes = charge.session.duration_min ?? (charge.end.getTime() - charge.start.getTime()) / 60_000;
  const color = CATEGORY_COLOR[charge.category];
  const time = (minute: number) => formatAppTime(charge.start.getTime() + minute * 60_000);
  const limit = (charge.session as ChargeSession & { charge_limit?: number | null }).charge_limit ?? null;

  const levelPoints = curve.map((point) => ({ x: point.minutes_elapsed!, y: point.soc_pct, tip: `${n0(point.soc_pct)}% · ${time(point.minutes_elapsed!)}` }));
  const powerPoints = curve.map((point) => ({ x: point.minutes_elapsed!, y: point.power_kw, tip: `${n1(point.power_kw)} kW · ${time(point.minutes_elapsed!)}` }));
  const xDomain: [number, number] | undefined = levelPoints.length ? [levelPoints[0]!.x, levelPoints.at(-1)!.x] : undefined;
  const xLabels = levelPoints.length > 1
    ? [[levelPoints[0]!.x, time(levelPoints[0]!.x), 'start'], [levelPoints.at(-1)!.x, time(levelPoints.at(-1)!.x), 'end']] as Array<[number, string, 'start' | 'end']>
    : [];
  const peakKw = Math.max(1, ...powerPoints.map((point) => point.y));
  const powerTop = charge.dc ? Math.ceil(peakKw / 50) * 50 : Math.ceil(peakKw / 4) * 4;
  const reportedPeak = charge.session.peak_power_kw;

  return (
    <div className="r-act-detail" data-charge-detail={charge.id}>
      {header}
      <div className="r-act-dbody">
        <h1 className="r-act-dtitle">{chargeTitle(charge)}</h1>
        <p className="muted">
          {formatAppDate(charge.start, { weekday: 'short', month: 'short', day: 'numeric', year: undefined })} · {formatAppTime(charge.start)}
          {charge.active ? ' · Charging now' : ` – ${formatAppTime(charge.end)}`}
        </p>
        <div className="r-act-hero" data-charge-hero>
          <div><p className="r-act-label">Added</p><p className="v num">+{n1(charge.kwh)} <small>kWh</small></p></div>
          <div>
            <p className="r-act-label">Battery</p>
            <p className="v num">{charge.socStart != null && charge.socEnd != null ? <>{n0(charge.socStart)}→{n0(charge.socEnd)}<small>%</small></> : '–'}</p>
          </div>
          <div>
            <p className="r-act-label">{charge.dc ? 'Peak power' : 'Duration'}</p>
            <p className="v num">{charge.dc ? (reportedPeak ? <>{n0(reportedPeak)} <small>kW</small></> : '–') : duration(minutes)}</p>
          </div>
          <div>
            <p className="r-act-label">Cost</p>
            <p className="v num" data-charge-cost>{charge.cost == null ? <span className="r-act-pill">Not set</span> : charge.cost === 0 ? 'Free' : usd(charge.cost)}</p>
          </div>
        </div>

        <h2 className="r-act-h2">Place</h2>
        <PlaceCard charge={charge} places={places} vehicleId={vehicleId} canManage={canManage} />

        <h2 className="r-act-h2">Cost</h2>
        <CostCard charge={charge} places={places} vehicleId={vehicleId} canManage={canManage} />

        <h2 className="r-act-h2">Charge level</h2>
        <section className="r-act-card pad">
          {curveQuery.isLoading ? <Skeleton className="h-36 w-full" /> : levelPoints.length > 1 ? (
            <ExpandableChart title="Charge level" render={(big) => <LineChart
              height={big ? 320 : 150}
              label={`Charge level from ${n0(charge.socStart ?? levelPoints[0]!.y)}% to ${n0(charge.socEnd ?? levelPoints.at(-1)!.y)}%`}
              points={levelPoints}
              yDomain={[0, 100]}
              ticks={[0, 50, 100]}
              tickFormat={(value) => `${value}%`}
              color={color}
              area
              reference={limit != null ? { value: limit, label: `Limit ${n0(limit)}%` } : null}
              xLabels={xLabels}
              endLabels={[`${n0(levelPoints[0]!.y)}%`, `${n0(levelPoints.at(-1)!.y)}%`]}
              testId={big ? 'charge-level-chart-big' : 'charge-level-chart'}
            />} />
          ) : <p className="r-act-empty">No samples.</p>}
        </section>

        <h2 className="r-act-h2">Charging power</h2>
        <section className="r-act-card pad">
          {curveQuery.isLoading ? <Skeleton className="h-28 w-full" /> : powerPoints.length > 1 ? (
            <ExpandableChart title={estimated ? 'Estimated charging power' : 'Charging power'} render={(big) => <LineChart
              label={estimated ? 'Estimated charging power' : 'Charging power'}
              points={powerPoints}
              height={big ? 320 : 120}
              yDomain={[0, powerTop]}
              ticks={[0, powerTop / 2, powerTop]}
              tickFormat={(value) => `${n0(value)} kW`}
              color={color}
              {...(xDomain ? { xDomain } : {})}
              xLabels={xLabels}
              testId={big ? 'charge-power-chart-big' : 'charge-power-chart'}
            />} />
          ) : <p className="r-act-empty">No samples.</p>}
          <p className="r-act-note" data-power-note>
            {estimated
              ? `Estimated from the battery-level change${capacity ? ` and a ${n1(capacity)} kWh pack` : ''}. The car does not report power samples for this charge.`
              : 'Power samples from the car.'}
            {reportedPeak ? ` Rivian reported a peak of ${n0(reportedPeak)} kW.` : ''}
          </p>
        </section>
      </div>
    </div>
  );
}

function PlaceCard({ charge, places, vehicleId, canManage }: {
  charge: ActivityCharge; places: Place[]; vehicleId: string; canManage: boolean;
}) {
  const tag = usePlaceTag(vehicleId);
  const [pending, setPending] = React.useState<PlaceCategory | null>(null);
  const [tried, setTried] = React.useState<PlaceCategory | null>(null);
  const shown = pending ?? charge.category;
  const missing = (category: PlaceCategory) => category !== 'public'
    && !places.some((place) => (category === 'home' ? place.is_home : place.is_work) || place.name.trim().toLocaleLowerCase() === (category === 'home' ? 'home' : 'office'));
  return (
    <section className="r-act-card pad" data-place-card>
      <div className="r-act-ctitle"><b>{shortPlace(charge.place)}</b><span className="muted">This charge only</span></div>
      <Segmented
        label="Tag this charge"
        value={shown}
        disabled={!canManage || tag.isPending}
        options={(['home', 'office', 'public'] as const).map((category) => ({ value: category, label: CATEGORY_LABEL[category] }))}
        buttonProps={(option) => ({ 'data-tag': option.value })}
        onChange={(value) => {
          const category = value as PlaceCategory;
          if (category === charge.category) return;
          setPending(category);
          setTried(category);
          tag.mutate({ sessionId: charge.id, category, scope: 'session' }, { onSettled: () => setPending(null) });
        }}
      />
      {tag.isError ? (
        <p className="r-act-note r-act-error" role="alert">
          {tried && missing(tried) ? `Set your ${CATEGORY_LABEL[tried]} rate in Settings first.` : 'The tag did not save. Try again.'}
        </p>
      ) : null}
      <p className="r-act-note">Choose Home, Office, or Other for this charge only — other charges are not changed. Home and Office use your rates in Settings unless you set a cost for this charge below.</p>
      {charge.placeApproximate ? <p className="r-act-note">{shortPlace(charge.place)} is the nearest town, not an address.</p> : null}
    </section>
  );
}

function CostCard({ charge, places, vehicleId, canManage }: { charge: ActivityCharge; places: Place[]; vehicleId: string; canManage: boolean }) {
  const update = useUpdateChargeSession(vehicleId);
  const manual = charge.session.cost_method === 'user_manual' && charge.session.cost_usd != null;
  const hasOverride = charge.session.cost_override_mode === 'manual' || charge.session.cost_override_mode === 'free';
  const [mode, setMode] = React.useState<CostEntryMode>('total');
  const [input, setInput] = React.useState(manual ? charge.session.cost_usd!.toFixed(2) : '');
  const [message, setMessage] = React.useState<string | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  React.useEffect(() => {
    setInput(manual ? charge.session.cost_usd!.toFixed(2) : '');
    setMessage(null);
    setError(null);
  }, [charge.id, manual, charge.session.cost_usd]);
  const entry = costEntry(mode, input, charge.kwh);
  const editable = canManage && !charge.active;
  const inputId = React.useId();

  const save = (event: React.FormEvent) => {
    event.preventDefault();
    setMessage(null);
    if (entry.total == null) { setError(entry.error ?? (mode === 'rate' ? 'Enter a price per kWh.' : 'Enter the total you paid.')); return; }
    setError(null);
    update.mutate({ sessionId: charge.id, cost_mode: 'manual', cost_usd: entry.total }, {
      onSuccess: () => setMessage(`Saved ${usd(entry.total!)}.`),
      onError: () => setError('The cost did not save. Try again.'),
    });
  };

  return (
    <section className="r-act-card pad" data-cost-card>
      <p className="r-act-why" data-cost-reason>{costReason(charge, places)}</p>
      {editable ? (
        <form className="r-act-cost-ed" onSubmit={save} noValidate>
          <Segmented
            label="How to enter the cost"
            value={mode}
            options={[{ value: 'total', label: 'Total paid' }, { value: 'rate', label: 'Price per kWh' }]}
            onChange={(value) => { setMode(value as CostEntryMode); setError(null); setMessage(null); }}
          />
          <div className="r-act-cost-row">
            <label className="sr-only" htmlFor={inputId}>{mode === 'rate' ? 'Price per kWh' : 'Total paid'}</label>
            <input
              id={inputId}
              inputMode="decimal"
              autoComplete="off"
              placeholder={mode === 'rate' ? '$ per kWh, e.g. 0.48' : '$ total, e.g. 18.40'}
              value={input}
              onChange={(event) => { setInput(event.target.value); setError(null); setMessage(null); }}
              aria-invalid={error ? true : undefined}
              data-cost-input
            />
            <button type="submit" className="r-act-btn" disabled={update.isPending}>Save</button>
          </div>
          <div className="r-act-confirm-actions">
            <Button
              type="button"
              size="sm"
              variant="secondary"
              className="min-h-11"
              disabled={update.isPending || charge.session.cost_override_mode === 'free'}
              onClick={() => {
                setError(null);
                setMessage(null);
                update.mutate({ sessionId: charge.id, cost_mode: 'free' }, {
                  onSuccess: () => { setInput(''); setMessage('Marked free ($0.00).'); },
                  onError: () => setError('The cost did not save. Try again.'),
                });
              }}
            >
              Mark free ($0)
            </Button>
            {hasOverride ? (
              <Button
                type="button"
                size="sm"
                variant="secondary"
                className="min-h-11"
                disabled={update.isPending}
                onClick={() => {
                  setError(null);
                  setMessage(null);
                  update.mutate({ sessionId: charge.id, cost_mode: 'automatic' }, {
                    onSuccess: () => { setInput(''); setMessage('Reverted to automatic rate.'); },
                    onError: () => setError('The cost did not save. Try again.'),
                  });
                }}
              >
                Use automatic rate
              </Button>
            ) : null}
          </div>
          {error ? <p className="r-act-note r-act-error" role="alert">{error}</p>
            : message ? <p className="r-act-note" role="status">{message}</p>
              : entry.preview ? <p className="r-act-note">{entry.preview}</p> : null}
        </form>
      ) : null}
    </section>
  );
}
