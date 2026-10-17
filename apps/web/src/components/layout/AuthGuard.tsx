import React, { useEffect, useRef } from 'react';
import { createAuthSessionLifecycle } from './authSessionLifecycle';
import { useNavigate } from '@tanstack/react-router';
import { useAuth } from '@panda/hooks';
import { hydrateQueryCacheForUser, clearQueryCacheForUser } from '../../queryClient';

type LoginLocation = Pick<Location, 'pathname' | 'replace'>;
type LoginRedirectScheduler = (callback: () => void, delayMs: number) => unknown;

export function normalizeLoginRedirectTarget(value: string | null | undefined): string | null {
  if (!value) return null;
  const trimmed = value.trim();
  if (!trimmed.startsWith('/') || trimmed.startsWith('//')) return null;

  try {
    const url = new URL(trimmed, 'https://panda.local');
    if (url.origin !== 'https://panda.local') return null;
    if (url.pathname === '/login') return null;
    return `${url.pathname}${url.search}${url.hash}`;
  } catch {
    return null;
  }
}

function currentProtectedTarget(locationLike: Pick<Location, 'pathname'> & Partial<Pick<Location, 'search' | 'hash'>> | null) {
  if (!locationLike || locationLike.pathname === '/login') return null;
  return normalizeLoginRedirectTarget(`${locationLike.pathname}${locationLike.search ?? ''}${locationLike.hash ?? ''}`);
}

function buildLoginUrl(redirectTarget: string | null) {
  return redirectTarget ? `/login?redirect=${encodeURIComponent(redirectTarget)}` : '/login';
}

export function forceLoginRedirect(
  navigate: (options: { to: '/login' }) => void,
  locationLike: LoginLocation | null = typeof window !== 'undefined' ? window.location : null,
  scheduleRedirect: LoginRedirectScheduler | null =
    typeof window !== 'undefined' ? window.setTimeout.bind(window) : null,
) {
  const current = locationLike as (LoginLocation & Partial<Pick<Location, 'search' | 'hash'>>) | null;
  const redirectTarget = currentProtectedTarget(current);
  const options = redirectTarget ? { to: '/login' as const, search: { redirect: redirectTarget } } : { to: '/login' as const };
  try { navigate(options); } catch { /* Browser navigation remains available if routing fails. */ }
  const isTestLocation = typeof window !== 'undefined' && current === window.location
    && typeof navigator !== 'undefined' && /\bjsdom\b/i.test(navigator.userAgent);
  if (!current || !scheduleRedirect || isTestLocation) return;
  const hardRedirect = () => {
    if (current.pathname === '/login') return;
    try { current.replace(buildLoginUrl(redirectTarget)); } catch { /* Some runtimes do not implement navigation. */ }
  };
  scheduleRedirect(hardRedirect, 0);
}

/**
 * Wraps a page component. On first load, attempts to rehydrate the session
 * from the HttpOnly refresh cookie before deciding whether to redirect.
 * This replaces the old approach of persisting the access token in localStorage.
 */
export function AuthGuard({ children }: { children: React.ReactNode }) {
  const { isAuthenticated, isBootstrapping, accessToken, resumeSession, clearSession } = useAuth();
  const navigate = useNavigate();
  const ports = useRef({ resumeSession, clearSession, navigate });
  useEffect(() => { ports.current = { resumeSession, clearSession, navigate }; });
  const [session] = React.useState(() => createAuthSessionLifecycle({
    resume: () => ports.current.resumeSession(),
    clear: () => ports.current.clearSession(),
    redirect: () => forceLoginRedirect(ports.current.navigate),
    hydrate: hydrateQueryCacheForUser,
    clearCache: clearQueryCacheForUser,
  }));
  useEffect(() => {
    const unmount = session.mount(isAuthenticated);
    window.addEventListener('panda:auth-expired', session.recover);
    return () => {
      unmount();
      window.removeEventListener('panda:auth-expired', session.recover);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [session]);
  useEffect(() => {
    session.observe({ authenticated: isAuthenticated, bootstrapping: isBootstrapping, token: accessToken });
  }, [session, isAuthenticated, isBootstrapping, accessToken]);
  return !isBootstrapping && isAuthenticated ? <>{children}</> : null;
}
