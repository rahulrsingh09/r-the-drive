import React from 'react';
import { useNavigate } from '@tanstack/react-router';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { api, queryKeys, useAuth, useMe, useRevokeOtherAuthSessions } from '@panda/hooks';
import { PASSWORD_MIN_LENGTH, PasswordRequirements } from '../../components/auth/PasswordRequirements';

const ROLE_LABEL: Record<string, string> = { super_user: 'Owner', admin: 'Admin', user: 'Member' };

/** Sign-in methods of this account, as one short phrase. */
export function rSignInMethodLabel(passwordConfigured: boolean | undefined, ssoLinked: boolean | undefined) {
  if (passwordConfigured !== false && ssoLinked) return 'Password and SSO';
  if (ssoLinked) return 'SSO';
  return 'Password';
}

export function RAccountCard() {
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const { clearSession, logout } = useAuth();
  const me = useMe();
  const identities = useQuery({ queryKey: queryKeys.auth.identities, queryFn: () => api.getOidcIdentities(), retry: false });
  const revokeOthers = useRevokeOtherAuthSessions();
  const [open, setOpen] = React.useState(false);
  const [current, setCurrent] = React.useState('');
  const [next, setNext] = React.useState('');
  const [confirm, setConfirm] = React.useState('');
  const [error, setError] = React.useState('');
  const [saving, setSaving] = React.useState(false);
  const [othersMessage, setOthersMessage] = React.useState('');
  const formId = React.useId();

  const email = me.data?.email ?? '';
  const role = ROLE_LABEL[me.data?.role ?? ''] ?? '';
  const passwordConfigured = identities.data?.password_configured ?? me.data?.password_configured;
  const method = rSignInMethodLabel(passwordConfigured, identities.data?.oidc_linked);

  async function changePassword(event: React.FormEvent) {
    event.preventDefault();
    if (next.length < PASSWORD_MIN_LENGTH || next !== confirm || !current) return;
    setSaving(true);
    setError('');
    try {
      await api.changePassword({ current_password: current, new_password: next });
      queryClient.clear();
      clearSession();
      navigate({ to: '/login', search: { password_changed: '1' } });
    } catch (failure) {
      const message = (failure as { detail?: { message?: string } }).detail?.message;
      setError(message ?? 'The password did not change. Check the current password and try again.');
    } finally {
      setSaving(false);
    }
  }

  async function signOut() {
    await logout();
    navigate({ to: '/login' });
  }

  function signOutOthers() {
    setOthersMessage('');
    revokeOthers.mutate(undefined, {
      onSuccess: (result) => {
        const count = result?.revoked;
        setOthersMessage(typeof count !== 'number' ? 'Signed out the other devices.'
          : count === 0 ? 'No other device was signed in.'
            : `Signed out ${count} other ${count === 1 ? 'device' : 'devices'}.`);
      },
    });
  }

  return (
    <section id="account" className="r-card r-pad r-rise" aria-labelledby="settings-account-title">
      <div className="r-ctitle"><h2 id="settings-account-title">Account</h2>{role && <span className="r-muted">{role}</span>}</div>
      <div className="r-acct">
        <div className="r-avatar" aria-hidden="true">{(email[0] ?? 'R').toUpperCase()}</div>
        <div className="r-acct-text">
          <b>Signed in</b>
          {email && <div className="r-acct-email">{email}</div>}
          <div className="r-muted r-acct-sub">
            {method}
          </div>
        </div>
      </div>
      <div className="r-btnrow">
        {passwordConfigured !== false && (
          <button type="button" className="r-btn r-btn-ghost" aria-expanded={open} aria-controls={formId} onClick={() => setOpen(value => !value)}>
            Change password
          </button>
        )}
        <button type="button" className="r-btn r-btn-ghost" onClick={signOutOthers}
          disabled={revokeOthers.isPending}>
          {revokeOthers.isPending ? 'Signing out…' : 'Sign out other devices'}
        </button>
        <button type="button" className="r-btn r-btn-danger" onClick={() => { void signOut(); }}>Sign out</button>
      </div>
      {othersMessage && <p className="r-form-ok r-gap-top" role="status">{othersMessage}</p>}
      {revokeOthers.isError && <p className="r-form-error r-gap-top" role="alert">
        {(revokeOthers.error as { detail?: { status?: number } } | null)?.detail?.status === 409
          ? 'This browser has no active session. Sign out, sign in again, then try again.'
          : 'The other devices are still signed in. Try again.'}
      </p>}
      {open && (
        <form id={formId} className="r-setform" onSubmit={changePassword}>
          <label>Current password<input type="password" autoComplete="current-password" value={current} onChange={event => setCurrent(event.target.value)} required /></label>
          <label>New password<input type="password" autoComplete="new-password" minLength={PASSWORD_MIN_LENGTH} value={next} onChange={event => setNext(event.target.value)} required /></label>
          <PasswordRequirements password={next} />
          <label>Confirm new password<input type="password" autoComplete="new-password" value={confirm} onChange={event => setConfirm(event.target.value)} required
            aria-invalid={confirm !== '' && confirm !== next} /></label>
          {confirm !== '' && confirm !== next && <p className="r-form-error" role="alert">The passwords do not match.</p>}
          {error && <p className="r-form-error" role="alert">{error}</p>}
          <button type="submit" className="r-btn" disabled={saving || !current || next.length < PASSWORD_MIN_LENGTH || next !== confirm}>
            {saving ? 'Saving…' : 'Save password'}
          </button>
          <p className="r-note">Changing your password signs out every session.</p>
        </form>
      )}
    </section>
  );
}
