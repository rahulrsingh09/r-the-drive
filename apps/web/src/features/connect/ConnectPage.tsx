import React from 'react';
import { useNavigate, useSearch } from '@tanstack/react-router';
import type { ConnectedRivianVehicle } from '@panda/types';
import { PageLayout, Button, Input, Card } from '@panda/ui/primitives';
import { AppLayout } from '../../components/layout/AppLayout';
import { ConnectedVehicleSuccess } from '../../components/connect/ConnectedVehicleSuccess';
import { formatVehicleName, useConnectionFlow } from './useConnectionFlow';
import { Car, Check, KeyRound, ShieldCheck, Zap } from 'lucide-react';

export function ConnectContent() {
  const navigate = useNavigate();
  const search = useSearch({ from: '/connect' });
  const refreshVehicleId = search.mode === 'refresh' ? search.vehicle_id : undefined;
  const { error, loading, vehicles, successVehicleName, successVehicleId, successWaitingMessage, refreshVehicle, submitCredentials, selectVehicle } = useConnectionFlow(refreshVehicleId, refreshVehicleId ?? 'new');
  const currentStep = successVehicleName ? 2 : loading ? 1 : 0;
  function handleSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const fields = new FormData(event.currentTarget);
    const email = String(fields.get('email') ?? '');
    void submitCredentials(email, String(fields.get('password') ?? ''), (challengeId) => {
      void navigate({ to: '/connect/otp', search: { challenge_id: challengeId, email, mode: search.mode, vehicle_id: refreshVehicleId } });
    });
  }

  return (
    <AppLayout activeKey="settings">
      <PageLayout
        title={refreshVehicle ? 'Refresh Rivian Login' : 'Add a Vehicle'}
        subtitle={
          refreshVehicle
            ? `Update encrypted credentials for ${refreshVehicle.display_name}.`
            : 'Connect your Rivian account so R:The Drive can securely prepare telemetry access.'
        }
        className="min-h-[calc(100vh-3rem)] justify-center [&>div:first-child]:justify-center [&>div:first-child>div]:text-center"
      >
        <div className="mx-auto w-full max-w-2xl">
          <Card padding="lg" className="shadow-lg">
            <ConnectProgress currentStep={currentStep} />

            {successVehicleName ? (
              <ConnectedVehicleSuccess
                vehicleId={successVehicleId}
                vehicleName={successVehicleName}
                initialWaitingMessage={successWaitingMessage}
                onOpenDashboard={() => navigate({ to: refreshVehicle ? '/settings' : '/' })}
              />
            ) : vehicles.length > 1 ? (
              <VehiclePicker
                vehicles={vehicles}
                loading={loading}
                error={error}
                onSelect={selectVehicle}
              />
            ) : (
              <>
                <div className="mt-8 flex items-center gap-3">
                  <div className="flex h-9 w-9 items-center justify-center rounded-lg bg-accent-muted">
                    <Zap className="h-4 w-4 text-accent" />
                  </div>
                  <div>
                    <p className="text-sm font-medium text-fg">Rivian Account</p>
                    <p className="text-xs text-fg-tertiary">
                      Most accounts continue through a one-time verification code.
                    </p>
                  </div>
                </div>

                <form onSubmit={handleSubmit} className="mt-5 flex flex-col gap-4">
                  <Input
                    label="Rivian Email"
                    type="email"
                    name="email"
                    autoComplete="section-rivian username"
                    placeholder="you@example.com"
                    required
                  />
                  <Input
                    label="Rivian Password"
                    type="password"
                    name="password"
                    autoComplete="section-rivian current-password"
                    placeholder="Password"
                    required
                  />
                  {error && <p className="text-xs text-status-danger">{error}</p>}
                  <Button
                    type="submit"
                    loading={loading}
                    iconLeft={<KeyRound className="h-4 w-4" />}
                    className="mt-1"
                  >
                    {loading ? 'Checking account' : 'Connect Account'}
                  </Button>
                </form>
              </>
            )}
          </Card>
        </div>
      </PageLayout>
    </AppLayout>
  );
}

const steps = [
  {
    label: 'Credentials',
    description: 'Sign in with the Rivian account that has access to the vehicle.',
    icon: KeyRound,
  },
  {
    label: 'Verification',
    description: 'If Rivian requests MFA, enter the email or authenticator code next.',
    icon: ShieldCheck,
  },
  {
    label: 'Vehicle saved',
    description: 'R:The Drive encrypts tokens and sets the first connected vehicle as default.',
    icon: Car,
  },
];

function ConnectProgress({ currentStep }: { currentStep: number }) {
  return (
    <div aria-label="Add vehicle progress">
      <div className="h-2 overflow-hidden rounded-full bg-bg-elevated">
        <div
          className="h-full rounded-full bg-accent transition-all duration-300"
          style={{ width: `${((currentStep + 1) / steps.length) * 100}%` }}
        />
      </div>
      <div className="mt-5 grid grid-cols-3 items-stretch gap-3">
        {steps.map((step, index) => {
          const Icon = step.icon;
          const active = index === currentStep;
          const complete = index < currentStep;
          return (
            <div
              key={step.label}
              className={`flex gap-3 rounded-lg border p-3 ${
                active || complete
                  ? 'border-accent/50 bg-accent-muted/20'
                  : 'border-border bg-bg-elevated/40'
              }`}
            >
              <div
                className={`mt-0.5 flex h-7 w-7 shrink-0 items-center justify-center rounded-lg ${
                  complete ? 'bg-accent text-fg-on-accent' : 'bg-bg-surface text-accent'
                }`}
              >
                {complete ? <Check className="h-4 w-4" /> : <Icon className="h-4 w-4" />}
              </div>
              <div>
                <p className="text-sm font-medium text-fg">{step.label}</p>
                <p className="mt-1 text-xs leading-5 text-fg-tertiary">{step.description}</p>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

function VehiclePicker({
  vehicles,
  loading,
  error,
  onSelect,
}: {
  vehicles: ConnectedRivianVehicle[];
  loading: boolean;
  error: string;
  onSelect: (vehicle: ConnectedRivianVehicle) => void;
}) {
  return (
    <div className="mt-8">
      <p className="text-sm font-medium text-fg">Choose a vehicle</p>
      <p className="mt-1 text-xs leading-5 text-fg-tertiary">
        Rivian returned multiple vehicles for this account. Pick the one to add first.
      </p>
      <div className="mt-4 grid gap-3">
        {vehicles.map((vehicle) => (
          <button
            key={vehicle.id}
            type="button"
            disabled={loading}
            onClick={() => onSelect(vehicle)}
            className="flex items-center justify-between rounded-lg border border-border bg-bg-elevated/40 p-4 text-left transition hover:border-accent/50 disabled:cursor-not-allowed disabled:opacity-60"
          >
            <span>
              <span className="block text-sm font-medium text-fg">
                {formatVehicleName(vehicle)}
              </span>
              <span className="mt-1 block text-xs text-fg-tertiary">
                {vehicle.vin ?? vehicle.id}
              </span>
            </span>
            <Car className="h-5 w-5 text-accent" />
          </button>
        ))}
      </div>
      {error && <p className="mt-4 text-xs text-status-danger">{error}</p>}
    </div>
  );
}
