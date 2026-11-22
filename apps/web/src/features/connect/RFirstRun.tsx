import React from 'react';
import { getVehicleArtworkFallback, useCurrentVehicleStatus } from '@panda/hooks';
import type { ConnectedRivianVehicle } from '@panda/types';
import { RMark } from '../r-experience/RMark';
import { RIconBack, RIconCheck } from '../r-experience/RShellIcons';
import { RRatesForm } from '../settings/RRatesCard';
import { formatVehicleName, type RConnected } from './useRivianConnect';
import './r-first-run.css';

export type RFirstRunStep = 'welcome' | 'connect' | 'code' | 'car' | 'rates' | 'done';

/** Steps of each flow. First run has all six (v5 trial); Reconnect has three. */
export function rFirstRunSteps(flow: 'first' | 'add' | 'refresh'): RFirstRunStep[] {
  if (flow === 'refresh') return ['connect', 'code', 'done'];
  if (flow === 'add') return ['connect', 'code', 'car', 'rates', 'done'];
  return ['welcome', 'connect', 'code', 'car', 'rates', 'done'];
}

/** Full-screen frame with the step dots and an optional back button. */
export function RFirstRunFrame({ steps, step, onBack, children }: {
  steps: RFirstRunStep[]; step: RFirstRunStep; onBack?: (() => void) | undefined; children: React.ReactNode;
}) {
  const index = Math.max(0, steps.indexOf(step));
  React.useEffect(() => { document.title = 'Set up · R'; }, []);
  return (
    <main className="r-fr" aria-label="Set up R">
      <div className="r-fr-in">
        <div className="r-fr-top">
          {onBack ? <button type="button" className="r-iconbtn" aria-label="Back" onClick={onBack}><RIconBack /></button> : <span />}
          <div className="r-fr-dots" role="img" aria-label={`Step ${index + 1} of ${steps.length}`}>
            {steps.map((name, i) => <i key={name} className={i === index ? 'on' : i < index ? 'done' : undefined} />)}
          </div>
          <span />
        </div>
        <div className="r-fr-body" key={step}>{children}</div>
      </div>
    </main>
  );
}

export function RWelcomeStep({ onNext }: { onNext: () => void }) {
  return (
    <>
      <div className="r-fr-hero">
        <div className="r-fr-logo"><RMark size={96} /></div>
        <h1>Your Rivian, understood.</h1>
        <p className="r-muted">Every drive, charge and dollar. Private to you, on your own server.</p>
      </div>
      <button type="button" className="r-btn r-btn-big" onClick={onNext} autoFocus>Get started</button>
    </>
  );
}

export function RConnectStep({ refresh, vehicleName, loading, error, onSubmit }: {
  refresh: boolean; vehicleName?: string | undefined; loading: boolean; error: string; onSubmit: (email: string, password: string) => void;
}) {
  return (
    <>
      <h1>{refresh ? 'Reconnect your Rivian account' : 'Connect your Rivian account'}</h1>
      <p className="r-muted">
        {refresh && vehicleName ? `Sign in again so R can keep reading ${vehicleName}. ` : 'R reads your car through your Rivian account. '}
        Your password goes to Rivian only, and R keeps just the session.
      </p>
      <form className="r-setform" onSubmit={(event) => {
        event.preventDefault();
        // Read the submitted fields, so values that the browser filled in are sent too.
        const fields = new FormData(event.currentTarget);
        onSubmit(String(fields.get('email') ?? ''), String(fields.get('password') ?? ''));
      }}>
        <label>Rivian email<input type="email" name="email" autoComplete="section-rivian username" required /></label>
        <label>Rivian password<input type="password" name="password" autoComplete="section-rivian current-password" required /></label>
        {error && <p className="r-form-error" role="alert">{error}</p>}
        <button type="submit" className="r-btn r-btn-big" disabled={loading}>{loading ? 'Checking your account…' : 'Send code'}</button>
      </form>
    </>
  );
}

export function RCodeStep({ email, loading, error, onSubmit }: {
  email?: string | undefined; loading: boolean; error: string; onSubmit: (code: string) => void;
}) {
  return (
    <>
      <h1>Enter the code</h1>
      <p className="r-muted">{email ? <>Rivian sent a code for <b className="r-fr-email">{email}</b> to your phone or email.</> : 'Rivian sent a code to your phone or email.'}</p>
      <form className="r-setform" onSubmit={(event) => {
        event.preventDefault();
        onSubmit(String(new FormData(event.currentTarget).get('otp') ?? '').trim());
      }}>
        <label>Verification code
          <input className="r-otp-input" type="text" name="otp" inputMode="numeric" pattern="[0-9]*" autoComplete="one-time-code" maxLength={8} required autoFocus />
        </label>
        {error && <p className="r-form-error" role="alert">{error}</p>}
        <button type="submit" className="r-btn r-btn-big" disabled={loading}>{loading ? 'Checking the code…' : 'Verify'}</button>
      </form>
    </>
  );
}

function CarArt({ vehicle }: { vehicle: ConnectedRivianVehicle }) {
  const art = getVehicleArtworkFallback(vehicle.model, 'vehicle-card');
  return <div className="r-fr-car">{art ? <img src={art} alt="" /> : <span className="r-fr-car-none" aria-hidden="true">R</span>}</div>;
}

const carLine = (vehicle: ConnectedRivianVehicle) => [vehicle.model, vehicle.model_year, vehicle.vin].filter(Boolean).join(' · ');

export function RCarStep({ vehicles, loading, error, onConfirm }: {
  vehicles: ConnectedRivianVehicle[]; loading: boolean; error: string; onConfirm: (vehicle: ConnectedRivianVehicle) => void;
}) {
  const [only] = vehicles;
  if (vehicles.length === 1 && only) {
    return (
      <>
        <h1>Is this your car?</h1>
        <CarArt vehicle={only} />
        <div className="r-fr-carname"><b>{formatVehicleName(only)}</b>{carLine(only) && <span className="r-muted">{carLine(only)}</span>}</div>
        {error && <p className="r-form-error" role="alert">{error}</p>}
        <button type="button" className="r-btn r-btn-big" disabled={loading} onClick={() => onConfirm(only)} autoFocus>
          {loading ? 'Adding your car…' : "Yes, that's my car"}
        </button>
      </>
    );
  }
  return (
    <>
      <h1>Which car is yours?</h1>
      <p className="r-muted">This Rivian account has {vehicles.length} cars. Choose the one R follows.</p>
      <div className="r-fr-cars">
        {vehicles.map(vehicle => (
          <button key={vehicle.id} type="button" className="r-fr-carpick" disabled={loading} onClick={() => onConfirm(vehicle)}>
            <CarArt vehicle={vehicle} />
            <span><b>{formatVehicleName(vehicle)}</b><span className="r-muted">{carLine(vehicle)}</span></span>
          </button>
        ))}
      </div>
      {error && <p className="r-form-error" role="alert">{error}</p>}
    </>
  );
}

export function RRatesStep({ onDone }: { onDone: () => void }) {
  return (
    <>
      <h1>What do you pay to charge?</h1>
      <p className="r-muted">R prices every charge from these. You can change them later in Settings. Office 0 means free.</p>
      <RRatesForm submitLabel="Finish" big onSaved={onDone} />
      <button type="button" className="r-tablelink r-fr-skip" onClick={onDone}>Skip for now</button>
    </>
  );
}

export function RDoneStep({ refresh, connected, onOpen }: { refresh: boolean; connected: RConnected; onOpen: () => void }) {
  const { data: status } = useCurrentVehicleStatus(connected.vehicleId);
  const health = status?.worker_health?.toLowerCase();
  const attention = health === 'error' || health === 'degraded' || health === 'stale';
  const message = connected.waitingMessage
    ?? (attention ? `${connected.vehicleName} is saved, but R cannot read it yet. Car → Data health shows why.`
      : refresh ? `${connected.vehicleName} is connected again. New drives and charges appear on their own.`
        : 'R is loading your Rivian history now. It takes about a minute. New drives and charges appear on their own.');
  return (
    <>
      <div className="r-fr-hero" aria-live="polite">
        <div className="r-fr-done"><RIconCheck /></div>
        <h1>{refresh ? 'Rivian is connected again' : "You're all set"}</h1>
        <p className={connected.waitingMessage || attention ? 'r-fr-warn' : 'r-muted'}>{message}</p>
      </div>
      <button type="button" className="r-btn r-btn-big" onClick={onOpen} autoFocus>{refresh ? 'Back to Settings' : 'Open R'}</button>
    </>
  );
}
