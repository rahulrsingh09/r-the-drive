import React from 'react';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ConnectOtpContent } from '../features/connect/ConnectOtpPage';

const mocks = vi.hoisted(() => ({ otp: vi.fn(), add: vi.fn(), setDefault: vi.fn(), navigate: vi.fn(), challenge: 'first' }));
vi.mock('@tanstack/react-router', () => ({ useNavigate: () => mocks.navigate, useSearch: () => ({ challenge_id: mocks.challenge, email: 'synthetic@example.invalid' }) }));
vi.mock('@panda/ui/primitives', async () => import('./mockPrimitives'));
vi.mock('../components/layout/AppLayout', () => ({ AppLayout: ({ children }: { children: React.ReactNode }) => <main>{children}</main> }));
vi.mock('@panda/hooks', () => ({
  api: { connectRivianOtp: mocks.otp, addVehicle: mocks.add },
  notifyVehicleCredentialsRefreshed: vi.fn(),
  useAuth: (select: (state: { setDefaultVehicleId: typeof mocks.setDefault }) => unknown) => select({ setDefaultVehicleId: mocks.setDefault }),
  useVehicles: () => ({ data: [] }),
  useCurrentVehicleStatus: () => ({ data: null }),
  queryKeys: { vehicles: { all: ['vehicles'], status: (id: string) => ['status', id] }, vehicle: { health: (id: string) => ['health', id] } },
}));

function renderOtp() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  return render(<QueryClientProvider client={client}><ConnectOtpContent /></QueryClientProvider>);
}
function pendingResult() {
  let resolve!: (value: unknown) => void;
  const promise = new Promise((yes) => { resolve = yes; });
  return { promise, resolve };
}
const vehicle = { id: 'synthetic-vehicle', name: 'Synthetic vehicle', model: 'R1S', vin: null };

beforeEach(() => { vi.clearAllMocks(); mocks.challenge = 'first'; });

describe('connection request ownership', () => {
  it('does not add a vehicle when OTP finishes after leaving the page', async () => {
    const result = pendingResult();
    mocks.otp.mockReturnValue(result.promise);
    const view = renderOtp();
    fireEvent.submit(screen.getByRole('button', { name: 'Verify and Connect' }).closest('form')!);
    await waitFor(() => expect(mocks.otp).toHaveBeenCalled());
    view.unmount();
    await act(async () => { result.resolve({ vehicles: [vehicle] }); await result.promise; });
    expect(mocks.add).not.toHaveBeenCalled();
    expect(mocks.setDefault).not.toHaveBeenCalled();
  });

  it('does not set the default vehicle when adding finishes after leaving the page', async () => {
    const added = pendingResult();
    mocks.otp.mockResolvedValue({ vehicles: [vehicle] });
    mocks.add.mockReturnValue(added.promise);
    const view = renderOtp();
    fireEvent.submit(screen.getByRole('button', { name: 'Verify and Connect' }).closest('form')!);
    await waitFor(() => expect(mocks.add).toHaveBeenCalled());
    view.unmount();
    await act(async () => { added.resolve({ vehicle_id: 'saved', telemetry_status: 'unchanged' }); await added.promise; });
    expect(mocks.setDefault).not.toHaveBeenCalled();
  });
});
