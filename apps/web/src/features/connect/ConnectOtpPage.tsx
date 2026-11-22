import React from 'react';
import { useNavigate, useSearch } from '@tanstack/react-router';
import { api } from '@panda/hooks';
import { connectErrorMessage } from '../../components/connect/connectError';
import { RCarStep, RCodeStep, RDoneStep, RFirstRunFrame, RRatesStep, rFirstRunSteps, type RFirstRunStep } from './RFirstRun';
import { useConnectFlow } from './ConnectPage';
import { useRivianConnect } from './useRivianConnect';

/** /connect/otp: the Rivian code, then the car, rates and done steps. */
export function ConnectOtpContent() {
  const navigate = useNavigate();
  const { challenge_id, email, mode, vehicle_id } = useSearch({ from: '/connect/otp' });
  const connect = useRivianConnect(mode, vehicle_id);
  const flow = useConnectFlow(connect.refreshVehicleId, connect.vehiclesLoaded, connect.firstRun) ?? 'add';
  const steps = rFirstRunSteps(flow);
  const [ratesDone, setRatesDone] = React.useState(false);
  const refresh = flow === 'refresh';
  const step: RFirstRunStep = connect.connected
    ? (refresh || ratesDone ? 'done' : 'rates')
    : connect.candidates.length ? 'car' : 'code';

  async function submit(code: string) {
    connect.setError('');
    connect.setLoading(true);
    try {
      const result = await api.connectRivianOtp(challenge_id, code);
      await connect.handleResult(result, 'Rivian verification succeeded, but no vehicles were returned for this account.');
    } catch (failure) {
      connect.reportError(connectErrorMessage(failure));
    } finally {
      connect.setLoading(false);
    }
  }

  const onBack = step === 'code'
    ? () => navigate({ to: '/connect', search: { mode, vehicle_id } })
    : undefined;

  return (
    <RFirstRunFrame steps={steps} step={step} onBack={onBack}>
      {step === 'code' && <RCodeStep email={email} loading={connect.loading} error={connect.error} onSubmit={(code) => { void submit(code); }} />}
      {step === 'car' && <RCarStep vehicles={connect.candidates} loading={connect.loading} error={connect.error}
        onConfirm={(vehicle) => { void connect.confirmVehicle(vehicle); }} />}
      {step === 'rates' && <RRatesStep onDone={() => setRatesDone(true)} />}
      {step === 'done' && connect.connected && <RDoneStep refresh={refresh} connected={connect.connected}
        onOpen={() => navigate({ to: refresh ? '/settings' : '/' })} />}
    </RFirstRunFrame>
  );
}
