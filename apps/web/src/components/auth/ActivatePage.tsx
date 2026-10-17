import React, { useEffect, useState } from 'react';
import { useNavigate } from '@tanstack/react-router';
import { useQuery } from '@tanstack/react-query';
import { api, useAuth } from '@panda/hooks';
import { Button, Input } from '@panda/ui/primitives';
import { RAuthBrand } from '../../features/r-experience/RAuthBrand';
import { PASSWORD_MIN_LENGTH, PasswordRequirements } from './PasswordRequirements';
import { useAuthForm } from './useAuthForm';

export function ActivatePage() {
  const navigate = useNavigate();
  const accept = useAuth((state) => state.acceptAccountInvitation);
  const [token] = useState(() => window.location.hash.replace(/^#/, ''));
  const { password, error, loading, ssoLoading, setPassword, setError, run } = useAuthForm(true);
  const invitation = useQuery({
    queryKey: ['account-invitation', token], queryFn: () => api.previewAccountInvitation(token),
    enabled: !!token, retry: false,
  });
  useEffect(() => {
    if (window.location.hash) window.history.replaceState(null, '', window.location.pathname);
  }, []);
  async function submit(event: React.FormEvent) {
    event.preventDefault();
    if (password.length < PASSWORD_MIN_LENGTH) {
      setError(`Use a password with at least ${PASSWORD_MIN_LENGTH} characters.`);
      return;
    }
    await run('password', async () => { await accept(token, password); navigate({ to: '/' }); },
      cause => cause instanceof Error ? cause.message : 'This activation link could not be used. Ask an administrator for a new one.');
  }
  async function startSso() {
    await run('sso', async () => {
      const response = await api.startAccountInvitationOidc(token);
      window.location.assign(response.authorization_url);
    }, cause => cause instanceof Error ? cause.message : 'Could not connect to SSO. Try the invitation link again.', true);
  }
  const preview = invitation.data;
  const allowed = new Set(preview?.auth_methods === 'sso' ? ['sso'] : preview?.auth_methods === 'password' ? ['password'] : ['password', 'sso']);
  const passwordAvailable = allowed.has('password') && preview?.password_available === true;
  const ssoAvailable = allowed.has('sso') && preview?.sso_available === true;

  return <div className="min-h-screen bg-bg-page flex items-center justify-center px-4">
    <div className="w-full max-w-sm">
      <RAuthBrand />
      <div className="bg-bg-glass backdrop-blur-md border border-border rounded-2xl p-6 shadow-xl">
        <p className="text-[11px] font-semibold text-fg-tertiary uppercase tracking-widest mb-4">Activate account</p>
        {!token || invitation.isError ? <p className="text-sm text-status-danger">This activation link is invalid, expired, or has already been used.</p> : invitation.isLoading ? <p className="text-sm text-fg-tertiary">Checking invitation…</p> : <div className="grid gap-4">
          <p className="text-sm text-fg-secondary">Activate the account for <span className="text-fg font-medium">{preview?.email}</span>.</p>
          {ssoAvailable && <div className="grid gap-2">
            <p className="text-xs text-fg-tertiary">Use your invited email address with the SSO provider.</p>
            <Button type="button" size="lg" className="w-full" loading={ssoLoading} onClick={() => void startSso()}>{preview?.button_label || 'Continue with SSO'}</Button>
          </div>}
          {passwordAvailable && <form onSubmit={submit} className="grid gap-4">
            {ssoAvailable && <p className="text-xs text-fg-tertiary">Or create a password</p>}
            <Input label="Password" type="password" value={password} onChange={(event) => setPassword(event.target.value)} placeholder="••••••••••••" minLength={PASSWORD_MIN_LENGTH} required autoComplete="new-password" />
            <PasswordRequirements password={password} />
            <Button type="submit" size="lg" className="w-full" loading={loading}>Activate with password</Button>
          </form>}
          {!passwordAvailable && !ssoAvailable && <p className="text-sm text-status-danger">This invitation’s sign-in method is currently unavailable. Ask an administrator to check authentication settings or send a new invitation.</p>}
          {error && <p role="alert" className="text-xs text-status-danger bg-status-danger/10 border border-status-danger/20 rounded-lg px-3 py-2">{error}</p>}
        </div>}
      </div>
    </div>
  </div>;
}
