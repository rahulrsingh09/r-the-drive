import { describe, expect, it, vi } from 'vitest';
import { createAuthSessionLifecycle } from '../components/layout/authSessionLifecycle';

function fixture() {
  const ports = { resume: vi.fn(), clear: vi.fn(), redirect: vi.fn(), hydrate: vi.fn(), clearCache: vi.fn() };
  return { ports, session: createAuthSessionLifecycle(ports) };
}
const token = (owner: string) => `e30.${btoa(JSON.stringify({ sub: owner }))}.synthetic`;

describe('web shell auth lifecycle boundaries', () => {
  it('blocks duplicate recovery, ignores its completion after unmount, and hydrates each owner once', async () => {
    const { ports, session } = fixture();
    let resolve!: (ok: boolean) => void;
    ports.resume.mockImplementation(() => new Promise<boolean>(done => { resolve = done; }));
    const unmount = session.mount(true);
    session.observe({ authenticated: true, bootstrapping: false, token: token('account-a') });
    session.observe({ authenticated: true, bootstrapping: false, token: token('account-a') });
    session.recover(); session.recover();
    await Promise.resolve();
    expect(ports.resume).toHaveBeenCalledTimes(1);
    expect(ports.hydrate).toHaveBeenCalledTimes(1);
    session.observe({ authenticated: false, bootstrapping: false, token: null });
    expect(ports.redirect).not.toHaveBeenCalled();
    unmount(); resolve(false);
    await Promise.resolve(); await Promise.resolve();
    expect(ports.clear).not.toHaveBeenCalled();
    expect(ports.clearCache).not.toHaveBeenCalled();
  });

  it('clears cache before session expiry redirect, and never redirects twice', async () => {
    const { ports, session } = fixture();
    const events: string[] = [];
    ports.resume.mockRejectedValue(new Error('synthetic failure'));
    ports.clearCache.mockImplementation(() => events.push('cache'));
    ports.clear.mockImplementation(() => events.push('session'));
    ports.redirect.mockImplementation(() => events.push('redirect'));
    session.mount(true);
    session.observe({ authenticated: true, bootstrapping: false, token: token('account-a') });
    session.recover();
    for (let i = 0; i < 5; i++) await Promise.resolve();
    session.observe({ authenticated: false, bootstrapping: false, token: null });
    expect(events).toEqual(['cache', 'session', 'redirect']);
  });
});
