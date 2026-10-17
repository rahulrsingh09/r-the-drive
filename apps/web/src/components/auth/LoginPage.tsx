import React from 'react';
import { useNavigate, useSearch } from '@tanstack/react-router';
import { useQuery } from '@tanstack/react-query';
import { api, queryKeys, useAuth } from '@panda/hooks';
import { Button, Input } from '@panda/ui/primitives';
import { RAuthBrand } from '../../features/r-experience/RAuthBrand';
import { Zap, Route, Battery } from 'lucide-react';
import { normalizeLoginRedirectTarget } from '../layout/AuthGuard';
import { PASSWORD_MIN_LENGTH, PasswordRequirements } from './PasswordRequirements';
import { emitAuthError } from '../feedback/toast';
import { useAuthForm } from './useAuthForm';
import { useLoginSession } from './useLoginSession';
import { loginFailureMessage } from './loginFailure';

export function LoginPage() {
  const navigate = useNavigate();
  const search = useSearch({ from: '/login' });
  const auth = useAuth();
  const { password, error, loading: passwordLoading, ssoLoading, setPassword, setError, run } = useAuthForm();
  const loading = passwordLoading || ssoLoading;
  const redirectTarget = normalizeLoginRedirectTarget(search.redirect);
  const setup = useQuery({ queryKey: queryKeys.auth.setup, queryFn: () => api.setup(), retry: false });
  const setupRequired = setup.data?.setup_required === true;
  const setupProofRequired = setup.data?.setup_proof_required === true;
  const setupProofAvailable = setup.data?.setup_proof_available === true;
  const authConfig = useQuery({
    queryKey: queryKeys.auth.config, queryFn: () => api.getAuthConfig(), retry: false,
    enabled: setup.isSuccess && !setupRequired,
  });
  const config = authConfig.data;
  const ssoReady = !setupRequired && config?.oidc_enabled === true && config.oidc_ready === true;
  const passwordEnabled = setupRequired || config?.password_login_enabled === true;
  const startSso = React.useCallback(() => run('sso', async () => {
    const response = await api.startOidc(redirectTarget ?? '/');
    window.location.assign(response.authorization_url);
  }, () => {
    emitAuthError('SSO sign-in failed', 'Single sign-on is temporarily unavailable.');
    return passwordEnabled
      ? 'Single sign-on is temporarily unavailable. Use your password or contact an administrator.'
      : 'Single sign-on is temporarily unavailable. Contact an administrator.';
  }), [run, redirectTarget, passwordEnabled]);
  const enter = React.useCallback(() => {
    navigate({ to: (redirectTarget ?? '/') as never, replace: true });
  }, [navigate, redirectTarget]);
  const restoringSession = useLoginSession({
    authenticated: auth.isAuthenticated, bootstrapping: auth.isBootstrapping,
    resume: auth.resumeSession, enter, resumeAllowed: search.password_changed !== '1',
    autoSsoAllowed: Boolean(config?.oidc_auto_login && ssoReady && !search.error && !search.password_changed),
    startSso,
  });

  if (restoringSession) {
    return (
      <div className="min-h-screen bg-bg-page flex items-center justify-center px-4">
        <p role="status" className="text-sm text-fg-secondary">Restoring your session…</p>
      </div>
    );
  }

  async function handleSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const fields = new FormData(event.currentTarget);
    const email = String(fields.get('email') ?? '');
    const submittedPassword = String(fields.get('password') ?? '');
    const proof = String(fields.get('setupToken') ?? '');
    await run('password', async () => {
      if (setupRequired && setupProofRequired && !setupProofAvailable) {
        setError('First-owner setup is not configured on this instance. Ask the administrator to set PANDA_SETUP_TOKEN and recreate the app without deleting the database.');
        return;
      }
      const destination = setupRequired ? '/connect' : redirectTarget ?? '/';
      const request = setupRequired
        ? auth.register(email, submittedPassword, setupProofRequired ? proof : undefined)
        : auth.login(email, submittedPassword);
      await request;
      navigate({ to: destination as never });
    }, cause => {
      const message = loginFailureMessage(cause);
      emitAuthError('Sign-in failed', message);
      return message;
    });
  }

  return (
    <div className="min-h-screen bg-bg-page flex items-center justify-center px-4 relative overflow-hidden">
      <div className="w-full max-w-sm relative z-10">
        <RAuthBrand />

        {/* Auth card */}
        <div className="bg-bg-glass backdrop-blur-md border border-border rounded-2xl p-6 shadow-xl">
          <p className="text-[11px] font-semibold text-fg-tertiary uppercase tracking-widest mb-5">
            {setupRequired ? 'Set up R' : 'Sign in'}
          </p>
          {search.password_changed === '1' && (
            <p role="status" className="mb-4 rounded-lg border border-status-positive/30 bg-status-positive/10 px-3 py-2 text-xs text-status-positive">
              Password changed. Sign in with your new password.
            </p>
          )}
          {search.error && (
            <p role="alert" className="mb-4 rounded-lg border border-status-warning/30 bg-status-warning/10 px-3 py-2 text-xs text-status-warning">
              {search.error === 'oidc_cancelled' || search.error === 'oidc_denied'
                ? 'Single sign-on was cancelled or denied.'
                : search.error === 'oidc_expired'
                  ? 'This single sign-on attempt expired.'
                  : 'Single sign-on could not be completed.'}{' '}
              {passwordEnabled ? 'You can try again or use your password.' : 'You can try again or contact an administrator.'}
            </p>
          )}

          {!setupRequired && !config && <p role="status" className="text-xs text-fg-secondary">Loading sign-in options…</p>}
          {ssoReady && <Button type="button" size="lg" className="w-full" loading={loading} onClick={startSso}>{config?.button_label || 'Sign in with SSO'}</Button>}
          {error && !passwordEnabled && <p role="alert" className="mt-4 text-xs text-status-danger bg-status-danger/10 border border-status-danger/20 rounded-lg px-3 py-2">{error}</p>}
          {ssoReady && passwordEnabled && <div className="my-1 flex items-center gap-3 text-[11px] text-fg-tertiary"><span className="h-px flex-1 bg-border" /><span>or</span><span className="h-px flex-1 bg-border" /></div>}
          {passwordEnabled && <form onSubmit={handleSubmit} className="flex flex-col gap-4">
            <Input
              label="Email"
              name="email"
              type="email"
              placeholder="you@example.com"
              required
              autoComplete="email"
            />
            <Input
              label="Password"
              name="password"
              type="password"
              onChange={(e) => setPassword(e.target.value)}
              placeholder="••••••••"
              required
              minLength={setupRequired ? PASSWORD_MIN_LENGTH : undefined}
              autoComplete={setupRequired ? 'new-password' : 'current-password'}
            />
            {setupRequired && setupProofRequired && setupProofAvailable && (
              <Input
                label="Instance setup token"
                name="setupToken"
                type="password"
                hint="Use the one-time PANDA_SETUP_TOKEN configured for this instance."
                autoComplete="off"
                required
              />
            )}
            {setupRequired && setupProofRequired && !setupProofAvailable && (
              <p role="alert" className="text-xs text-status-danger bg-status-danger/10 border border-status-danger/20 rounded-lg px-3 py-2">
                This production instance needs a setup token before the first owner can be created. Configure PANDA_SETUP_TOKEN, then recreate the app without deleting its database.
              </p>
            )}
            {setupRequired && <PasswordRequirements password={password} />}
            {error && (
              <p role="alert" className="text-xs text-status-danger bg-status-danger/10 border border-status-danger/20 rounded-lg px-3 py-2">
                {error}
              </p>
            )}
            <Button type="submit" loading={loading} size="lg" className="mt-1 w-full">
              {setupRequired ? 'Create owner account' : 'Sign in'}
            </Button>
          </form>}
          {!passwordEnabled && !ssoReady && config && <p role="status" className="text-xs text-fg-secondary">Sign-in is temporarily unavailable. Ask an administrator to restore an authentication method.</p>}
          {!setupRequired && <p className="mt-5 pt-5 border-t border-border text-center text-xs text-fg-tertiary">Need access? Ask an administrator for an activation link.</p>}
        </div>

        {/* Feature callouts */}
        <div className="mt-8 grid grid-cols-3 gap-3">
          {[
            { icon: Route, label: 'Trip analytics', sub: 'Every drive logged' },
            { icon: Zap, label: 'Charge history', sub: 'Sessions & cost' },
            { icon: Battery, label: 'Battery health', sub: 'SOC over time' },
          ].map(({ icon: Icon, label, sub }) => (
            <div key={label} className="text-center">
              <div className="flex justify-center mb-1.5">
                <Icon className="h-3.5 w-3.5 text-accent/70" />
              </div>
              <p className="text-[11px] font-medium text-fg-secondary">{label}</p>
              <p className="text-[10px] text-fg-tertiary mt-0.5">{sub}</p>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
