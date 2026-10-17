import React from 'react';
import { act, renderHook } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import { useDashboardEditButtonPreference } from '../components/dashboard/useDashboardEditButtonPreference';
import { settingsEntryProps } from '../lib/settingsEntryProps';

beforeEach(() => localStorage.clear());

it('ignores legacy dashboard edit preferences without reading, writing or subscribing to storage', () => {
  localStorage.setItem('rm-show-dashboard-edit-button:a', 'true');
  const read = vi.spyOn(Storage.prototype, 'getItem');
  const write = vi.spyOn(Storage.prototype, 'setItem');
  const subscribe = vi.spyOn(window, 'addEventListener');
  const { result, rerender } = renderHook(({ user }) => useDashboardEditButtonPreference(user), { initialProps: { user: 'a' }, wrapper: ({ children }) => <React.StrictMode>{children}</React.StrictMode> });
  expect(result.current[0]).toBe(false);
  rerender({ user: 'b' });
  expect(result.current[0]).toBe(false);
  act(() => result.current[1](true));
  act(() => window.dispatchEvent(new StorageEvent('storage')));
  expect(result.current[0]).toBe(false);
  rerender({ user: 'a' });
  expect(result.current[0]).toBe(false);
  expect(read).not.toHaveBeenCalled();
  expect(write).not.toHaveBeenCalled();
  expect(subscribe.mock.calls.some(([event]) => event === 'storage' || event === 'rm-dashboard-edit-button-preference-change')).toBe(false);
  vi.restoreAllMocks();
});

it('prioritizes recovery-password feedback over SSO linking and callback errors', () => {
  expect(settingsEntryProps({ section: 'account', password: 'set', oidc: 'linked', error: 'oidc_failed' })).toEqual({
    initialSection: 'account', oidcFeedback: 'Recovery password set. Other refresh sessions were revoked.', oidcFeedbackKind: 'success',
  });
  expect(settingsEntryProps({ oidc: 'linked', error: 'oidc_failed' })).toEqual({ oidcFeedback: 'SSO identity linked.', oidcFeedbackKind: 'success' });
  expect(settingsEntryProps({})).toEqual({});
});
