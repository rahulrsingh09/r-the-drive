import { afterEach, expect, it, vi } from 'vitest';
import { api } from '../../../../packages/hooks/src/api';
import { useAuth, recordExplicitLogoutIntent, consumeExplicitLogoutIntent } from '../../../../packages/hooks/src/useAuth';

afterEach(() => { vi.restoreAllMocks(); useAuth.getState().clearSession(); });

it('rejects a login response that completes after explicit session clearing', async () => {
  let complete!: (value: { access_token: string; default_vehicle_id: null; expires_in: number }) => void;
  vi.spyOn(api, 'login').mockImplementation(() => new Promise((resolve) => { complete = resolve; }));
  const login = useAuth.getState().login('synthetic@example.invalid', 'test-only');
  useAuth.getState().clearSession();
  complete({ access_token: 'late-token', default_vehicle_id: null, expires_in: 900 });
  await expect(login).rejects.toMatchObject({ code: 'SESSION_CHANGED' });
  expect(useAuth.getState().accessToken).toBeNull();
});

it('records and consumes logout intent once even without usable storage', () => {
  vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => { throw new Error('unavailable'); });
  recordExplicitLogoutIntent();
  expect(consumeExplicitLogoutIntent()).toBe(true);
  expect(consumeExplicitLogoutIntent()).toBe(false);
});

it('persists only non-sensitive preference fields and reads JWT subject safely', () => {
  const token = `header.${btoa(JSON.stringify({ sub: 'synthetic-subject' }))}.signature`;
  useAuth.getState().setTokens(token, 'vehicle');
  expect(useAuth.getState().userId).toBe('synthetic-subject');
  expect(JSON.parse(localStorage.getItem('rm-auth')!).state).toEqual({ defaultVehicleId: 'vehicle', activeVehicleId: null, userId: 'synthetic-subject' });
  useAuth.getState().setTokens('invalid', null);
  expect(useAuth.getState().userId).toBeNull();
});
