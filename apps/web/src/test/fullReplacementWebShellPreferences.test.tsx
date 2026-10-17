import React from 'react';
import { act, renderHook } from '@testing-library/react';
import { beforeEach, expect, it } from 'vitest';
import { useDashboardEditButtonPreference } from '../components/dashboard/useDashboardEditButtonPreference';
import { settingsEntryProps } from '../lib/settingsEntryProps';

beforeEach(() => localStorage.clear());

it('keeps dashboard edit visibility scoped to the current account and updates subscribers', () => {
  localStorage.setItem('rm-show-dashboard-edit-button:a', 'true');
  const { result, rerender } = renderHook(({ user }) => useDashboardEditButtonPreference(user), { initialProps: { user: 'a' }, wrapper: ({ children }) => <React.StrictMode>{children}</React.StrictMode> });
  expect(result.current[0]).toBe(true);
  rerender({ user: 'b' });
  expect(result.current[0]).toBe(false);
  act(() => result.current[1](true));
  expect(localStorage.getItem('rm-show-dashboard-edit-button:b')).toBe('true');
  act(() => { localStorage.setItem('rm-show-dashboard-edit-button:b', 'false'); window.dispatchEvent(new StorageEvent('storage')); });
  expect(result.current[0]).toBe(false);
  rerender({ user: 'a' });
  expect(result.current[0]).toBe(true);
});

it('prioritizes recovery-password feedback over SSO linking and callback errors', () => {
  expect(settingsEntryProps({ section: 'account', password: 'set', oidc: 'linked', error: 'oidc_failed' })).toEqual({
    initialSection: 'account', oidcFeedback: 'Recovery password set. Other refresh sessions were revoked.', oidcFeedbackKind: 'success',
  });
  expect(settingsEntryProps({ oidc: 'linked', error: 'oidc_failed' })).toEqual({ oidcFeedback: 'SSO identity linked.', oidcFeedbackKind: 'success' });
  expect(settingsEntryProps({})).toEqual({});
});
