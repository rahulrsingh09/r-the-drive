import React from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { api, queryKeys, useMe } from '@panda/hooks';
import type { AuthenticationSettings, AuthenticationSettingsUpdate } from '@panda/types';
import { RIconInfo } from '../r-experience/RShellIcons';

type Setting = { value?: unknown; source?: string } | undefined;
const valueOf = (setting: unknown) => (setting && typeof setting === 'object' && 'value' in setting ? (setting as Setting)?.value : undefined);
const fromEnvironment = (setting: unknown) => Boolean(setting && typeof setting === 'object' && (setting as Setting)?.source === 'environment');
const text = (setting: unknown) => (typeof valueOf(setting) === 'string' ? (valueOf(setting) as string) : '');

export interface RSignInState {
  passwordOn: boolean;
  ssoOn: boolean;
  /** Issuer, client ID and client secret are saved. */
  ssoConfigured: boolean;
  /** The provider passed the last test. */
  ssoTested: boolean;
  /** This account can sign in with SSO. */
  accountLinked: boolean;
}

/**
 * Switch rules. Never both off. Password sign-in turns off only after SSO is on, tested,
 * and linked to this account. SSO turns on only after it is set up and tested.
 */
export function rSignInRules(state: RSignInState) {
  const ssoWorks = state.ssoOn && state.ssoTested && state.accountLinked;
  return {
    canTurnPasswordOff: state.passwordOn && ssoWorks,
    canTurnSsoOn: !state.ssoOn && state.ssoConfigured && state.ssoTested,
    canTurnSsoOff: state.ssoOn && state.passwordOn,
    passwordOffReason: !state.ssoOn ? 'Turn on SSO first.'
      : !state.ssoTested ? 'Test SSO first.'
        : !state.accountLinked ? 'Connect SSO to this account first.' : null,
  };
}

export function rSignInState(settings: AuthenticationSettings, linked: boolean): RSignInState {
  return {
    passwordOn: valueOf(settings.password_login_enabled) !== false,
    ssoOn: valueOf(settings.oidc_enabled) === true,
    ssoConfigured: Boolean(text(settings.issuer_url) && text(settings.client_id) && settings.client_secret?.configured),
    ssoTested: Boolean(settings.last_validation_at),
    accountLinked: linked,
  };
}

function Switch({ checked, label, sub, disabled, onToggle, hint }: {
  checked: boolean; label: string; sub: string; disabled: boolean; onToggle: () => void; hint?: string | null;
}) {
  const hintId = React.useId();
  return (
    <div className="r-trow">
      <div><div>{label}</div><div className="r-trow-sub">{sub}</div>{hint && <div id={hintId} className="r-trow-sub">{hint}</div>}</div>
      <button type="button" className="r-switch" role="switch" aria-checked={checked} aria-label={label}
        aria-disabled={disabled || undefined} {...(hint ? { 'aria-describedby': hintId } : {})}
        onClick={() => { if (!disabled) onToggle(); }}><i /></button>
    </div>
  );
}

function SsoSetupForm({ settings, onSaved }: { settings: AuthenticationSettings; onSaved: (message: string, error?: boolean) => void }) {
  const client = useQueryClient();
  const [issuer, setIssuer] = React.useState(text(settings.issuer_url));
  const [clientId, setClientId] = React.useState(text(settings.client_id));
  const [secret, setSecret] = React.useState('');
  const [label, setLabel] = React.useState(text(settings.button_label) || 'Sign in with SSO');
  const [publicUrl, setPublicUrl] = React.useState(text(settings.public_base_url));
  const save = useMutation({
    mutationFn: async () => {
      const body: AuthenticationSettingsUpdate = {};
      const put = <K extends keyof AuthenticationSettingsUpdate>(key: K, value: AuthenticationSettingsUpdate[K]) => {
        if (!fromEnvironment(settings[key as keyof AuthenticationSettings]) && value !== text(settings[key as keyof AuthenticationSettings])) body[key] = value;
      };
      put('issuer_url', issuer.trim());
      put('client_id', clientId.trim());
      put('button_label', label.trim() || 'Sign in with SSO');
      put('public_base_url', publicUrl.trim());
      if (secret.trim() && settings.client_secret.source !== 'environment') body.client_secret = secret;
      const saved = Object.keys(body).length ? await api.updateAuthenticationSettings(body) : settings;
      client.setQueryData(queryKeys.auth.authenticationSettings, saved);
      await api.testAuthenticationSettings();
    },
    onSuccess: () => {
      setSecret('');
      onSaved('SSO works. You can now turn it on.');
    },
    onError: (error) => onSaved((error as { detail?: { message?: string } }).detail?.message
      ?? 'The SSO test failed. Check the issuer URL, client ID and client secret.', true),
    onSettled: () => {
      void client.invalidateQueries({ queryKey: queryKeys.auth.authenticationSettings });
      void client.invalidateQueries({ queryKey: queryKeys.auth.config });
    },
  });
  const env = (key: keyof AuthenticationSettings) => fromEnvironment(settings[key]);
  return (
    <form className="r-setform" onSubmit={(event) => { event.preventDefault(); save.mutate(); }}>
      <label>Issuer URL<input value={issuer} onChange={e => setIssuer(e.target.value)} placeholder="https://login.example.com" disabled={env('issuer_url')} inputMode="url" /></label>
      <label>Client ID<input value={clientId} onChange={e => setClientId(e.target.value)} disabled={env('client_id')} /></label>
      <label>Client secret<input type="password" value={secret} onChange={e => setSecret(e.target.value)} autoComplete="off"
        placeholder={settings.client_secret.configured ? 'Saved. Enter a new secret to replace it.' : ''} disabled={settings.client_secret.source === 'environment'} /></label>
      <label>Button label<input value={label} onChange={e => setLabel(e.target.value)} disabled={env('button_label')} /></label>
      <label>Public address of R<input value={publicUrl} onChange={e => setPublicUrl(e.target.value)} placeholder="https://r.example.com" disabled={env('public_base_url')} inputMode="url" /></label>
      {settings.callback_url && <p className="r-note">Callback URL: <code className="r-select-all">{settings.callback_url}</code></p>}
      <button type="submit" className="r-btn r-btn-ghost" disabled={save.isPending || !issuer.trim() || !clientId.trim() || (!secret.trim() && !settings.client_secret.configured)}>
        {save.isPending ? 'Testing…' : 'Save and test connection'}
      </button>
    </form>
  );
}

export function RSignInCard() {
  const client = useQueryClient();
  const me = useMe();
  const isOwner = me.data?.role === 'super_user';
  const settings = useQuery({
    queryKey: queryKeys.auth.authenticationSettings,
    queryFn: () => api.getAuthenticationSettings(),
    enabled: isOwner,
    retry: false,
  });
  const identities = useQuery({ queryKey: queryKeys.auth.identities, queryFn: () => api.getOidcIdentities(), retry: false });
  const [setupOpen, setSetupOpen] = React.useState(false);
  const [message, setMessage] = React.useState<{ text: string; error: boolean } | null>(null);
  const update = useMutation({
    mutationFn: (body: AuthenticationSettingsUpdate) => api.updateAuthenticationSettings(body),
    onSuccess: (data) => {
      client.setQueryData(queryKeys.auth.authenticationSettings, data);
      void client.invalidateQueries({ queryKey: queryKeys.auth.config });
      setMessage({ text: 'Sign-in methods saved.', error: false });
    },
    onError: (error) => setMessage({ text: (error as { detail?: { message?: string } }).detail?.message ?? 'The sign-in methods did not change. Try again.', error: true }),
  });
  const link = useMutation({
    mutationFn: () => api.startOidcLink('/settings?section=sign-in'),
    onSuccess: (result) => window.location.assign(result.authorization_url),
  });

  const linked = identities.data?.oidc_linked === true;
  const canLink = !linked && identities.data?.oidc_link_available === true && identities.data?.oidc_link_allowed !== false;

  let body: React.ReactNode;
  if (!isOwner) {
    body = (
      <>
        <Switch checked={identities.data?.password_configured !== false} label="Password sign-in" sub="Email and password" disabled onToggle={() => undefined} />
        <Switch checked={linked} label="Single sign-on (SSO)" sub={linked ? 'Linked to this account' : 'Not linked'} disabled onToggle={() => undefined} />
        {canLink && <button type="button" className="r-tablelink" onClick={() => link.mutate()} disabled={link.isPending}>Connect SSO to this account</button>}
        <p className="r-note">The owner of this R server chooses the sign-in methods.</p>
      </>
    );
  } else if (settings.isLoading) {
    body = <p className="r-note" role="status">Loading sign-in methods…</p>;
  } else if (settings.isError || !settings.data) {
    body = <p className="r-form-error" role="alert">The sign-in methods did not load. Reload the page to try again.</p>;
  } else {
    const data = settings.data;
    const state = rSignInState(data, linked);
    const rules = rSignInRules(state);
    const passwordEnv = fromEnvironment(data.password_login_enabled);
    const ssoEnv = fromEnvironment(data.oidc_enabled);
    const ssoSub = state.ssoOn ? (state.ssoTested ? 'On and tested' : 'On, not tested')
      : state.ssoConfigured ? (state.ssoTested ? 'Set up and tested' : 'Set up, not tested') : 'Not set up yet';
    body = (
      <>
        <Switch checked={state.passwordOn} label="Password sign-in" sub="Email and password"
          disabled={update.isPending || passwordEnv || (state.passwordOn ? !rules.canTurnPasswordOff : false)}
          hint={passwordEnv ? 'Set by the server environment.' : state.passwordOn && !rules.canTurnPasswordOff ? rules.passwordOffReason : null}
          onToggle={() => update.mutate({ password_login_enabled: !state.passwordOn })} />
        <Switch checked={state.ssoOn} label="Single sign-on (SSO)" sub={ssoSub}
          disabled={update.isPending || ssoEnv || (state.ssoOn ? !rules.canTurnSsoOff : !rules.canTurnSsoOn)}
          hint={ssoEnv ? 'Set by the server environment.' : state.ssoOn && !state.passwordOn ? 'Turn on password sign-in first.' : !state.ssoOn && !rules.canTurnSsoOn ? 'Set up and test SSO first.' : null}
          onToggle={() => update.mutate({ oidc_enabled: !state.ssoOn })} />
        <button type="button" className="r-tablelink" aria-expanded={setupOpen} onClick={() => setSetupOpen(value => !value)}>
          {setupOpen ? 'Hide SSO setup' : state.ssoConfigured ? 'Change SSO setup' : 'Set up SSO'}
        </button>
        {canLink && state.ssoOn && <button type="button" className="r-tablelink r-tablelink-next" onClick={() => link.mutate()} disabled={link.isPending}>Connect SSO to this account</button>}
        {setupOpen && <SsoSetupForm settings={data} onSaved={(text, error) => setMessage({ text, error: Boolean(error) })} />}
      </>
    );
  }

  return (
    <section id="sign-in" className="r-card r-pad r-rise" style={{ '--i': 1 } as React.CSSProperties} aria-labelledby="settings-sign-in-title">
      <div className="r-ctitle"><h2 id="settings-sign-in-title">Sign-in methods</h2><span className="r-muted">keep at least one on</span></div>
      {body}
      {message && <p className={message.error ? 'r-form-error r-gap-top' : 'r-form-ok r-gap-top'} role={message.error ? 'alert' : 'status'}>{message.text}</p>}
      <div className="r-callout r-gap-top"><RIconInfo /><span>Turn off password sign-in only after SSO works. If both are off, nobody can sign in.</span></div>
    </section>
  );
}
