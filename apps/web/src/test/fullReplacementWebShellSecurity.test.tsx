import React from 'react';
import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { normalizeLoginRedirectTarget } from '../components/layout/AuthGuard';
import { PasswordRequirements } from '../components/auth/PasswordRequirements';
import { resolveVehicleOnlineState } from '../components/layout/vehicleConnection';

describe('login redirect canonicalization', () => {
  it.each([
    [null, null], [undefined, null], ['', null], ['  ', null],
    ['/', '/'], [' /charging?view=table#curve ', '/charging?view=table#curve'],
    ['/a/../battery', '/battery'], ['/a/../login', null], ['/login#anything', null],
    ['/login/', '/login/'], ['/LOGIN', '/LOGIN'], ['/%6cogin', '/%6cogin'],
    ['//panda.local/battery', null], ['//example.invalid', null],
    ['https://panda.local/battery', null], ['javascript:alert(1)', null], ['battery', null],
    ['/\\example.invalid/battery', null], ['/\\panda.local/battery', '/battery'],
    ['/a\\b', '/a/b'], ['/a\\..\\login', null], ['\\battery', null],
    ['///panda.local/battery', null], ['/\n/example.invalid', null],
    ['/\n/panda.local/battery', '/battery'], ['/a\tb', '/ab'],
    ['/%2f%2fexample.invalid', '/%2f%2fexample.invalid'], ['/%5cexample.invalid', '/%5cexample.invalid'],
    ['/battery?redirect=https://example.invalid#x', '/battery?redirect=https://example.invalid#x'],
    ['/\\user:pass@panda.local/battery', '/battery'], ['/\\panda.local:444/battery', null],
    ['/\\[invalid', null], ['/battery%ZZ', '/battery%ZZ'],
  ])('normalizes %j to %j', (input, expected) => {
    expect(normalizeLoginRedirectTarget(input)).toBe(expected);
  });
});

describe('password progress classification', () => {
  it.each([
    ['', 'text-fg-tertiary', 'circle'],
    ['x', 'text-status-danger', 'circle'],
    ['x'.repeat(11), 'text-status-danger', 'circle'],
    ['x'.repeat(12), 'text-status-positive', 'check'],
    ['x'.repeat(30), 'text-status-positive', 'check'],
    ['😀'.repeat(6), 'text-status-positive', 'check'],
  ])('keeps the count, class and icon for %j', (password, color, icon) => {
    render(<PasswordRequirements password={password} />);
    const status = screen.getByRole('status');
    expect(status).toHaveClass(color);
    expect(status).toHaveAttribute('aria-label', `Password requires at least 12 characters. ${password.length} entered.`);
    expect(status).toHaveTextContent(`${password.length}/12`);
    expect(status.querySelector(`.lucide-${icon}`)).not.toBeNull();
  });
});

it('keeps vehicle connection priority for every readiness, transport and health combination', () => {
  for (const liveVehicleId of [null, 'v']) {
    for (const vehicleSelectionReady of [false, true]) {
      for (const connected of [false, true]) {
        for (const connectionState of ['failed', 'connected', 'connecting']) {
          for (const feedHealthIssue of [null, { key: 'x', title: 'x', message: 'x' }]) {
            const expected = liveVehicleId === null ? 'offline'
              : !vehicleSelectionReady || !connected || connectionState === 'failed' ? 'connecting'
                : feedHealthIssue ? 'unhealthy' : 'online';
            expect(resolveVehicleOnlineState({ liveVehicleId, vehicleSelectionReady, connected, connectionState, feedHealthIssue })).toBe(expected);
          }
        }
      }
    }
  }
});
