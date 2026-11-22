import React from 'react';
import { useNavigate, useSearch } from '@tanstack/react-router';
import { api } from '@panda/hooks';
import { connectErrorMessage } from '../../components/connect/connectError';
import {
  RCarStep, RConnectStep, RDoneStep, RFirstRunFrame, RRatesStep, RWelcomeStep, rFirstRunSteps, type RFirstRunStep,
} from './RFirstRun';
import { useRivianConnect } from './useRivianConnect';

export type RConnectFlow = 'first' | 'add' | 'refresh';

/** Fixes the flow once the vehicle list is known, so adding the car does not change the steps. */
export function useConnectFlow(refreshVehicleId: string | undefined, vehiclesLoaded: boolean, firstRun: boolean): RConnectFlow | null {
  const flow = React.useRef<RConnectFlow | null>(null);
  if (flow.current === null && (refreshVehicleId || vehiclesLoaded)) flow.current = refreshVehicleId ? 'refresh' : firstRun ? 'first' : 'add';
  return flow.current;
}

/** /connect: welcome and Rivian sign-in, then the car, rates and done steps when Rivian sends no code. */
export function ConnectContent() {
  const navigate = useNavigate();
  const search = useSearch({ from: '/connect' });
  const connect = useRivianConnect(search.mode, search.vehicle_id);
  const flow = useConnectFlow(connect.refreshVehicleId, connect.vehiclesLoaded, connect.firstRun) ?? 'add';
  const steps = rFirstRunSteps(flow);
  const [started, setStarted] = React.useState(false);
  const [ratesDone, setRatesDone] = React.useState(false);
  const refresh = flow === 'refresh';

  const step: RFirstRunStep = connect.connected
    ? (refresh || ratesDone ? 'done' : 'rates')
    : connect.candidates.length ? 'car'
      : flow === 'first' && !started ? 'welcome' : 'connect';

  async function submit(email: string, password: string) {
    connect.setError('');
    connect.setLoading(true);
    try {
      const result = await api.connectRivian(email, password);
      if (result.requires_otp && result.challenge_id) {
        navigate({
          to: '/connect/otp',
          search: { challenge_id: result.challenge_id, email, mode: search.mode, vehicle_id: connect.refreshVehicleId },
        });
        return;
      }
      await connect.handleResult(result, 'Rivian sign-in succeeded, but no vehicles were returned for this account.');
    } catch (failure) {
      connect.reportError(connectErrorMessage(failure));
    } finally {
      connect.setLoading(false);
    }
  }

  const onBack = step === 'connect'
    ? (flow === 'first' ? () => setStarted(false) : () => navigate({ to: '/settings' }))
    : undefined;

  return (
    <RFirstRunFrame steps={steps} step={step} onBack={onBack}>
      {step === 'welcome' && <RWelcomeStep onNext={() => setStarted(true)} />}
      {step === 'connect' && <RConnectStep refresh={refresh} vehicleName={connect.refreshVehicle?.display_name}
        loading={connect.loading} error={connect.error} onSubmit={(email, password) => { void submit(email, password); }} />}
      {step === 'car' && <RCarStep vehicles={connect.candidates} loading={connect.loading} error={connect.error}
        onConfirm={(vehicle) => { void connect.confirmVehicle(vehicle); }} />}
      {step === 'rates' && <RRatesStep onDone={() => setRatesDone(true)} />}
      {step === 'done' && connect.connected && <RDoneStep refresh={refresh} connected={connect.connected}
        onOpen={() => navigate({ to: refresh ? '/settings' : '/' })} />}
    </RFirstRunFrame>
  );
}
