import React from 'react';
import { render, screen, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import userEvent from '@testing-library/user-event';
import { vi, describe, it, expect, beforeEach } from 'vitest';

vi.mock('../../components/layout/AuthGuard', () => ({
  AuthGuard: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));

const routerMocks = vi.hoisted(() => ({
  navigate: vi.fn(),
  search: { challenge_id: 'challenge-123', email: 'driver@example.com' } as Record<string, string>,
}));
const apiMocks = vi.hoisted(() => ({
  connectRivian: vi.fn(),
  connectRivianOtp: vi.fn(),
  addVehicle: vi.fn(),
  refreshVehicleCredentials: vi.fn(),
  updatePlace: vi.fn(),
  notifyVehicleCredentialsRefreshed: vi.fn(),
  setDefaultVehicleId: vi.fn(),
  places: [] as unknown[],
}));

function apiError(code: string) {
  return Object.assign(new Error(code), { code });
}

vi.mock('@tanstack/react-router', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@tanstack/react-router')>();
  return { ...actual, useNavigate: () => routerMocks.navigate, useSearch: () => routerMocks.search };
});

vi.mock('@panda/hooks', () => ({
  api: {
    connectRivian: apiMocks.connectRivian,
    connectRivianOtp: apiMocks.connectRivianOtp,
    addVehicle: apiMocks.addVehicle,
    refreshVehicleCredentials: apiMocks.refreshVehicleCredentials,
    updatePlace: apiMocks.updatePlace,
  },
  notifyVehicleCredentialsRefreshed: apiMocks.notifyVehicleCredentialsRefreshed,
  invalidateChargingData: vi.fn(),
  getVehicleArtworkFallback: (model: string) => (model ? `/art/${model}.webp` : null),
  queryKeys: {
    vehicle: { health: (vehicleId: string) => ['vehicles', 'health', vehicleId] },
    vehicles: { all: ['vehicles'], status: (vehicleId: string) => ['vehicles', 'status', vehicleId] },
  },
  useAuth: (selector: (state: { setDefaultVehicleId: (vehicleId: string) => void }) => unknown) =>
    selector({ setDefaultVehicleId: apiMocks.setDefaultVehicleId }),
  useVehicles: () => ({
    isSuccess: true,
    data: routerMocks.search.mode === 'refresh'
      ? [{ id: 'local-vehicle-1', rivian_vehicle_id: 'rivian-vehicle-1', display_name: 'Launch Green' }]
      : [],
  }),
  useCurrentVehicleStatus: () => ({ data: null }),
  useSavedPlaces: () => ({ data: apiMocks.places, isLoading: false, isError: false }),
}));

import { ConnectContent } from '../connect';
import { ConnectOtpContent } from '../connect.otp';

const launchGreen = { id: 'rivian-vehicle-1', name: 'Launch Green', vin: '7FCTGAAL0NN000001', model: 'R1T', model_year: 2022 };
const forest = { id: 'rivian-vehicle-2', name: 'Forest R1S', vin: '7PDSGABL0NN000002', model: 'R1S', model_year: 2023 };
const connected = (vehicles: unknown[]) => ({ status: 'connected', requires_otp: false, challenge_id: null, vehicle_id: null, vehicles });

function renderWithQueryClient(ui: React.ReactElement) {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  return render(<QueryClientProvider client={queryClient}>{ui}</QueryClientProvider>);
}

async function signIn(user: ReturnType<typeof userEvent.setup>) {
  await user.type(screen.getByLabelText('Rivian email'), 'driver@example.com');
  await user.type(screen.getByLabelText('Rivian password'), 'secret123');
  await user.click(screen.getByRole('button', { name: 'Send code' }));
}

beforeEach(() => {
  vi.clearAllMocks();
  apiMocks.places = [];
  routerMocks.search = { challenge_id: 'challenge-123', email: 'driver@example.com' };
});

describe('ConnectContent (first run)', () => {
  it('opens on the welcome step and moves to Rivian sign-in', async () => {
    const user = userEvent.setup();
    renderWithQueryClient(<ConnectContent />);
    expect(screen.getByRole('heading', { name: 'Your Rivian, understood.' })).toBeInTheDocument();
    expect(screen.getByRole('img', { name: 'Step 1 of 6' })).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Get started' }));
    expect(screen.getByRole('heading', { name: 'Connect your Rivian account' })).toBeInTheDocument();
    expect(screen.getByRole('img', { name: 'Step 2 of 6' })).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Back' }));
    expect(screen.getByRole('button', { name: 'Get started' })).toBeInTheDocument();
  });

  it('routes accounts that need a code to the code step', async () => {
    apiMocks.connectRivian.mockResolvedValue({ status: 'otp_required', requires_otp: true, challenge_id: 'challenge-9', vehicle_id: null, vehicles: [] });
    const user = userEvent.setup();
    renderWithQueryClient(<ConnectContent />);
    await user.click(screen.getByRole('button', { name: 'Get started' }));
    await signIn(user);
    await waitFor(() => expect(routerMocks.navigate).toHaveBeenCalledWith({
      to: '/connect/otp',
      search: { challenge_id: 'challenge-9', email: 'driver@example.com', mode: undefined, vehicle_id: undefined },
    }));
    expect(apiMocks.connectRivian).toHaveBeenCalledWith('driver@example.com', 'secret123');
  });

  it('explains when Rivian rejects the entered credentials', async () => {
    apiMocks.connectRivian.mockRejectedValue(apiError('RIVIAN_CREDENTIALS_REJECTED'));
    const user = userEvent.setup();
    renderWithQueryClient(<ConnectContent />);
    await user.click(screen.getByRole('button', { name: 'Get started' }));
    await signIn(user);
    expect(await screen.findByText('Rivian did not accept that email or password. Check both and try again.')).toBeInTheDocument();
  });

  it('asks "Is this your car?", adds it, asks for rates, then finishes', async () => {
    apiMocks.connectRivian.mockResolvedValue(connected([launchGreen]));
    apiMocks.addVehicle.mockResolvedValue({ vehicle_id: 'local-vehicle-1' });
    const user = userEvent.setup();
    renderWithQueryClient(<ConnectContent />);
    await user.click(screen.getByRole('button', { name: 'Get started' }));
    await signIn(user);
    expect(await screen.findByRole('heading', { name: 'Is this your car?' })).toBeInTheDocument();
    expect(screen.getByText('Launch Green')).toBeInTheDocument();
    expect(apiMocks.addVehicle).not.toHaveBeenCalled();
    await user.click(screen.getByRole('button', { name: "Yes, that's my car" }));
    await waitFor(() => expect(apiMocks.addVehicle).toHaveBeenCalledWith({
      rivian_vehicle_id: 'rivian-vehicle-1', name: 'Launch Green', model: 'R1T', vin: '7FCTGAAL0NN000001',
    }));
    expect(apiMocks.setDefaultVehicleId).toHaveBeenCalledWith('local-vehicle-1');
    expect(await screen.findByRole('heading', { name: 'What do you pay to charge?' })).toBeInTheDocument();
    expect(screen.getByText('No Home place is set up yet.')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Finish' }));
    expect(await screen.findByRole('heading', { name: "You're all set" })).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Open R' }));
    expect(routerMocks.navigate).toHaveBeenCalledWith({ to: '/' });
  });

  it('lets the owner choose when the account has more than one car', async () => {
    apiMocks.connectRivian.mockResolvedValue(connected([launchGreen, forest]));
    apiMocks.addVehicle.mockResolvedValue({ vehicle_id: 'local-vehicle-2' });
    const user = userEvent.setup();
    renderWithQueryClient(<ConnectContent />);
    await user.click(screen.getByRole('button', { name: 'Get started' }));
    await signIn(user);
    expect(await screen.findByRole('heading', { name: 'Which car is yours?' })).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: /Forest R1S/ }));
    await waitFor(() => expect(apiMocks.addVehicle).toHaveBeenCalledWith(expect.objectContaining({ rivian_vehicle_id: 'rivian-vehicle-2' })));
  });

  it('shows an error when Rivian returns no cars', async () => {
    apiMocks.connectRivian.mockResolvedValue(connected([]));
    const user = userEvent.setup();
    renderWithQueryClient(<ConnectContent />);
    await user.click(screen.getByRole('button', { name: 'Get started' }));
    await signIn(user);
    expect(await screen.findByText('Rivian sign-in succeeded, but no vehicles were returned for this account.')).toBeInTheDocument();
  });

  it('explains a temporary secure-session failure while saving the car', async () => {
    apiMocks.connectRivian.mockResolvedValue(connected([launchGreen]));
    apiMocks.addVehicle.mockRejectedValue(apiError('RIVIAN_SESSION_UNAVAILABLE'));
    const user = userEvent.setup();
    renderWithQueryClient(<ConnectContent />);
    await user.click(screen.getByRole('button', { name: 'Get started' }));
    await signIn(user);
    await user.click(await screen.findByRole('button', { name: "Yes, that's my car" }));
    expect(await screen.findByRole('alert')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: "Yes, that's my car" })).toBeEnabled();
  });
});

describe('ConnectContent (Reconnect)', () => {
  beforeEach(() => {
    routerMocks.search = { mode: 'refresh', vehicle_id: 'local-vehicle-1' };
  });

  it('skips the welcome step, refreshes the credentials and the live socket', async () => {
    apiMocks.connectRivian.mockResolvedValue(connected([launchGreen]));
    apiMocks.refreshVehicleCredentials.mockResolvedValue({ telemetry_status: 'connected', telemetry_error: null });
    const user = userEvent.setup();
    renderWithQueryClient(<ConnectContent />);
    expect(screen.getByRole('heading', { name: 'Reconnect your Rivian account' })).toBeInTheDocument();
    expect(screen.getByRole('img', { name: 'Step 1 of 3' })).toBeInTheDocument();
    await signIn(user);
    await waitFor(() => expect(apiMocks.refreshVehicleCredentials).toHaveBeenCalledWith('local-vehicle-1', 'rivian-vehicle-1'));
    expect(apiMocks.notifyVehicleCredentialsRefreshed).toHaveBeenCalledWith('local-vehicle-1');
    expect(await screen.findByRole('heading', { name: 'Rivian is connected again' })).toBeInTheDocument();
    expect(screen.getByText('Launch Green is connected again. New drives and charges appear on their own.')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Back to Settings' }));
    expect(routerMocks.navigate).toHaveBeenCalledWith({ to: '/settings' });
  });

  it('keeps the reconnect successful while car data is still loading', async () => {
    apiMocks.connectRivian.mockResolvedValue(connected([launchGreen]));
    apiMocks.refreshVehicleCredentials.mockResolvedValue({ telemetry_status: 'waiting', telemetry_error: null });
    const user = userEvent.setup();
    renderWithQueryClient(<ConnectContent />);
    await signIn(user);
    expect(await screen.findByText('Rivian accepted the new sign-in. R is still waiting for car data and keeps trying.')).toBeInTheDocument();
  });

  it('refuses a Rivian account that does not include this car', async () => {
    apiMocks.connectRivian.mockResolvedValue(connected([forest]));
    const user = userEvent.setup();
    renderWithQueryClient(<ConnectContent />);
    await signIn(user);
    expect(await screen.findByText('That Rivian account does not include this vehicle.')).toBeInTheDocument();
    expect(apiMocks.refreshVehicleCredentials).not.toHaveBeenCalled();
  });
});

describe('ConnectOtpContent', () => {
  it('submits the code, confirms the car and adds it', async () => {
    apiMocks.connectRivianOtp.mockResolvedValue(connected([forest]));
    apiMocks.addVehicle.mockResolvedValue({ vehicle_id: 'local-vehicle-2' });
    const user = userEvent.setup();
    renderWithQueryClient(<ConnectOtpContent />);
    expect(screen.getByRole('heading', { name: 'Enter the code' })).toBeInTheDocument();
    expect(screen.getByText('driver@example.com')).toBeInTheDocument();
    await user.type(screen.getByLabelText('Verification code'), '123456');
    await user.click(screen.getByRole('button', { name: 'Verify' }));
    await waitFor(() => expect(apiMocks.connectRivianOtp).toHaveBeenCalledWith('challenge-123', '123456'));
    await user.click(await screen.findByRole('button', { name: "Yes, that's my car" }));
    await waitFor(() => expect(apiMocks.addVehicle).toHaveBeenCalledWith(expect.objectContaining({ rivian_vehicle_id: 'rivian-vehicle-2' })));
    expect(await screen.findByRole('heading', { name: 'What do you pay to charge?' })).toBeInTheDocument();
  });

  it('explains when Rivian rejects the code and keeps the form', async () => {
    apiMocks.connectRivianOtp.mockRejectedValue(apiError('RIVIAN_OTP_REJECTED'));
    const user = userEvent.setup();
    renderWithQueryClient(<ConnectOtpContent />);
    await user.type(screen.getByLabelText('Verification code'), '000000');
    await user.click(screen.getByRole('button', { name: 'Verify' }));
    expect(await screen.findByText('Rivian did not accept that verification code. Check it and try again.')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Verify' })).toBeEnabled();
  });

  it('goes back to sign-in with the same mode', async () => {
    routerMocks.search = { challenge_id: 'c', mode: 'refresh', vehicle_id: 'local-vehicle-1' };
    const user = userEvent.setup();
    renderWithQueryClient(<ConnectOtpContent />);
    await user.click(screen.getByRole('button', { name: 'Back' }));
    expect(routerMocks.navigate).toHaveBeenCalledWith({ to: '/connect', search: { mode: 'refresh', vehicle_id: 'local-vehicle-1' } });
  });

  it('shows an error when the code works but Rivian returns no cars', async () => {
    apiMocks.connectRivianOtp.mockResolvedValue(connected([]));
    const user = userEvent.setup();
    renderWithQueryClient(<ConnectOtpContent />);
    await user.type(screen.getByLabelText('Verification code'), '123456');
    await user.click(screen.getByRole('button', { name: 'Verify' }));
    expect(await screen.findByText('Rivian verification succeeded, but no vehicles were returned for this account.')).toBeInTheDocument();
  });
});
