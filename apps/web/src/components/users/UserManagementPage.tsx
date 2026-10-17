import { accountEditorTransition, emptyAccountEditor } from './accountEditor';
import { emptyInvitation, invitationDialogTransition, invitationDraftTransition } from './invitationDraft';
import React from 'react';
import { useFeatureOperation } from '../../features/featureOperation';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  Ban,
  Check,
  Clipboard,
  MailPlus,
  Pencil,
  Trash2,
  UserRound,
  UsersRound,
  X,
} from 'lucide-react';
import { api, useAuth, useAuthReady, useMe } from '@panda/hooks';
import type { AdminVehicleOption, AuthConfigResponse, InvitationAuthMethods, UserRole } from '@panda/types';
import {
  Badge,
  Button,
  Card,
  CardContent,
  EmptyState,
  PageLayout,
  SelectPicker,
  Tooltip,
} from '@panda/ui/primitives';
import { AppLayout } from '../layout/AppLayout';
import { formatAppDateTime } from '@panda/ui/lib/dateTime';

type Section = 'users' | 'invitations';
type Confirmation =
  | { kind: 'delete-user'; userId: string; email: string }
  | { kind: 'revoke-account-invitation'; invitationId: string; email: string }
  | { kind: 'revoke-vehicle-invitation'; userId: string; invitationId: string; vehicleName: string };

const CONTROL_CLASS = 'h-9 rounded-lg border border-border bg-bg-elevated px-3 text-sm text-fg outline-none transition-colors hover:border-border-strong focus:border-accent focus:ring-1 focus:ring-accent';
const MODAL_OVERLAY_CLASS =
  'fixed inset-0 z-50 flex items-center justify-center overflow-y-auto overflow-x-hidden bg-bg-page/80 p-2 backdrop-blur-sm sm:p-3';
const MODAL_PANEL_CLASS =
  'w-full max-h-[calc(100dvh-1rem)] overflow-y-auto rounded-xl border border-border bg-bg-surface p-5 shadow-lg';

function emitToast(title: string, message: string, variant: 'error' | 'success' | 'info' = 'info') {
  window.dispatchEvent(new CustomEvent('panda:toast', { detail: { title, message, variant } }));
}

function errorMessage(error: unknown) {
  return error instanceof Error && error.message ? error.message : 'Please try again.';
}

function invitationStatus(invitation: { accepted_at: string | null; revoked_at: string | null }) {
  if (invitation.accepted_at) return 'accepted';
  if (invitation.revoked_at) return 'revoked';
  return 'pending';
}

function availableInviteMethods(config: AuthConfigResponse | undefined): InvitationAuthMethods[] {
  const methods: Array<{ method: 'password' | 'sso' | 'both'; enabled: boolean }> = [
    { method: 'both', enabled: Boolean(config?.password_login_enabled && config.oidc_enabled && config.oidc_ready) },
    { method: 'password', enabled: Boolean(config?.password_login_enabled) },
    { method: 'sso', enabled: Boolean(config?.oidc_enabled && config.oidc_ready) },
  ];
  const available: Array<'password' | 'sso' | 'both'> = [];
  for (const entry of methods) if (entry.enabled) available.push(entry.method);
  return available;
}

const AUTH_METHOD_LABELS: Record<InvitationAuthMethods, string> = {
  password: 'Password only',
  sso: 'SSO only',
  both: 'Password and SSO',
};

function IconAction({
  label,
  children,
  variant = 'secondary',
  onClick,
  disabled = false,
}: {
  label: string;
  children: React.ReactNode;
  variant?: 'secondary' | 'danger';
  onClick: () => void;
  disabled?: boolean;
}) {
  return (
    <Tooltip content={label}>
      <Button
        type="button"
        variant={variant}
        aria-label={label}
        className="h-9 w-9 shrink-0 px-0"
        disabled={disabled}
        onClick={onClick}
      >
        {children}
      </Button>
    </Tooltip>
  );
}

function ConfirmationDialog({
  confirmation,
  onCancel,
  onConfirm,
  pending,
}: {
  confirmation: Confirmation;
  onCancel: () => void;
  onConfirm: () => void;
  pending: boolean;
}) {
  const copy = confirmation.kind === 'delete-user'
    ? {
      title: `Delete ${confirmation.email}?`,
      description: 'This permanently removes the account and its membership access.',
      action: 'Delete account',
    }
    : confirmation.kind === 'revoke-account-invitation'
      ? {
        title: `Revoke invitation for ${confirmation.email}?`,
        description: 'The recipient will no longer be able to activate this account invitation.',
        action: 'Revoke invitation',
      }
      : {
        title: `Revoke invitation for ${confirmation.vehicleName}?`,
        description: 'The recipient will no longer be able to accept this vehicle invitation.',
        action: 'Revoke invitation',
      };

  return (
    <div className={MODAL_OVERLAY_CLASS}>
      <div
        role="dialog"
        aria-modal="true"
        aria-label={copy.title}
        className={`${MODAL_PANEL_CLASS} sm:max-w-sm`}
      >
        <h2 className="text-base font-semibold text-fg">{copy.title}</h2>
        <p className="mt-2 text-sm text-fg-tertiary">{copy.description}</p>
        <div className="mt-5 flex justify-end gap-2">
          <Button type="button" variant="secondary" size="md" onClick={onCancel}>Cancel</Button>
          <Button type="button" variant="danger" size="md" loading={pending} onClick={onConfirm}>{copy.action}</Button>
        </div>
      </div>
    </div>
  );
}

function InviteUserDialog({
  email,
  setEmail,
  vehicleIds,
  setVehicleIds,
  vehicleOptions,
  authMethods,
  setAuthMethods,
  availableMethods,
  authOptionsLoading,
  authOptionsError,
  activationLink,
  pending,
  onClose,
  onSubmit,
  onCopy,
}: {
  email: string;
  setEmail: (email: string) => void;
  vehicleIds: string[];
  setVehicleIds: (vehicleIds: string[]) => void;
  vehicleOptions: AdminVehicleOption[];
  authMethods: InvitationAuthMethods;
  setAuthMethods: (methods: InvitationAuthMethods) => void;
  availableMethods: InvitationAuthMethods[];
  authOptionsLoading: boolean;
  authOptionsError: boolean;
  activationLink: string | null;
  pending: boolean;
  onClose: () => void;
  onSubmit: () => void;
  onCopy: () => void;
}) {
  const [{ copied, step }, dialogEvent] = React.useReducer(invitationDialogTransition, { copied: false, step: 'email' });

  return (
    <div className={MODAL_OVERLAY_CLASS}>
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Invite user"
        className={`${MODAL_PANEL_CLASS} sm:max-w-md`}
      >
        <div className="flex items-start justify-between gap-4">
          <div>
            <h2 className="text-base font-semibold text-fg">Invite user</h2>
            <p className="mt-1 text-sm text-fg-tertiary">Create an account invitation and optionally grant vehicle access. The activation link is shown once.</p>
          </div>
          <IconAction label="Close invite dialog" onClick={onClose}><X className="h-4 w-4" /></IconAction>
        </div>
        {activationLink ? (
          <div className="mt-5 grid gap-2">
            <label className="text-xs font-medium text-fg-secondary" htmlFor="activation-link">Activation link</label>
            <div className="flex gap-2">
              <input id="activation-link" readOnly value={activationLink} className={`${CONTROL_CLASS} min-w-0 flex-1 font-mono text-xs`} />
              <IconAction label={copied ? 'Activation link copied' : 'Copy activation link'} onClick={() => { onCopy(); dialogEvent('copied'); }}>
                {copied ? <Check className="h-4 w-4" /> : <Clipboard className="h-4 w-4" />}
              </IconAction>
            </div>
            <p className="text-xs text-fg-tertiary" aria-live="polite">{copied ? 'Activation link copied. It cannot be recovered later.' : 'Copy this link before closing. It cannot be recovered later.'}</p>
          </div>
        ) : (
          <form className="mt-5 grid gap-4" onSubmit={(event) => { event.preventDefault(); if (step === 'email') dialogEvent('continue'); else onSubmit(); }}>
            {step === 'email' ? (
              <div className="grid gap-4">
                <div className="grid gap-1.5">
                  <label className="text-sm font-medium text-fg-secondary" htmlFor="invite-email">Email address</label>
                  <input id="invite-email" autoFocus value={email} onChange={(event) => setEmail(event.target.value)} placeholder="email@example.com" className={`${CONTROL_CLASS} w-full`} />
                </div>
                <div className="grid gap-1.5">
                  <label className="text-sm font-medium text-fg-secondary" htmlFor="invite-auth-methods">Sign-in methods</label>
                  {availableMethods.length > 0 ? <SelectPicker id="invite-auth-methods" aria-label="Sign-in methods" value={authMethods} onChange={setAuthMethods} disabled={availableMethods.length === 1} options={availableMethods.map((method) => ({ value: method, label: AUTH_METHOD_LABELS[method] }))} /> : <p role="status" className="text-sm text-fg-tertiary">{authOptionsLoading ? 'Checking sign-in options…' : authOptionsError ? 'Could not load sign-in options. Close this dialog and try again.' : 'No sign-in method is available. Check the authentication settings.'}</p>}
                  {availableMethods.length > 0 && <p className="text-xs text-fg-tertiary">{authMethods === 'sso' ? 'The recipient must use the invited email address with the SSO provider. They cannot create a password.' : authMethods === 'password' ? 'The recipient creates a password and cannot connect SSO to this account.' : 'The recipient can activate with either method and add the other later.'}</p>}
                </div>
              </div>
            ) : (
              <div className="grid gap-1.5">
                <span className="text-sm font-medium text-fg-secondary">Vehicle access</span>
                <div className="grid gap-2" role="group" aria-label="Vehicle access">
                  <p className="text-xs text-fg-tertiary">Choose any vehicles to grant viewer access after activation. Leave all unchecked for no vehicle access.</p>
                  {vehicleOptions.map((vehicle) => { const checked = vehicleIds.includes(vehicle.id); return <label key={vehicle.id} className="flex min-w-0 items-start gap-3 rounded-lg border border-border px-3 py-2 text-sm text-fg transition-colors hover:border-border-strong has-[:focus-visible]:border-accent has-[:focus-visible]:ring-1 has-[:focus-visible]:ring-accent"><input type="checkbox" checked={checked} onChange={() => setVehicleIds(checked ? vehicleIds.filter((id) => id !== vehicle.id) : [...vehicleIds, vehicle.id])} className="mt-0.5 h-4 w-4 shrink-0 accent-accent" /><span className="min-w-0"><span className="block truncate">{vehicle.display_name}</span><span className="block text-xs text-fg-tertiary">{vehicle.model}</span></span></label>; })}
                  {vehicleOptions.length === 0 ? <p className="text-xs text-fg-tertiary">No vehicles are available.</p> : null}
                </div>
              </div>
            )}
            <div className="flex justify-end gap-2">
              <Button type="button" variant="secondary" size="md" onClick={step === 'email' ? onClose : () => dialogEvent('back')}>{step === 'email' ? 'Cancel' : 'Back'}</Button>
              <Button type="submit" size="md" iconLeft={step === 'vehicle' ? <MailPlus className="h-4 w-4" /> : undefined} loading={pending} disabled={!email.trim() || availableMethods.length === 0}>{step === 'email' ? 'Continue' : 'Create invitation'}</Button>
            </div>
          </form>
        )}
      </div>
    </div>
  );
}

export function UserManagementPage() {
  const queryClient = useQueryClient();
  const action = useFeatureOperation();
  const { accessToken } = useAuth();
  const authReady = useAuthReady();
  const me = useMe();
  const isPrivileged = me.data?.role === 'admin' || me.data?.role === 'super_user';
  const isSuperUser = me.data?.role === 'super_user';

  const [section, setSection] = React.useState<Section>('users');
  const [search, setSearch] = React.useState('');
  const [selectedUserId, setSelectedUserId] = React.useState<string | null>(null);
  const [accountEditor, accountEvent] = React.useReducer(accountEditorTransition, emptyAccountEditor);
  const { draft: accountDraft, editing: isEditingAccount, tab: detailTab } = accountEditor;
  const [grantVehicleId, setGrantVehicleId] = React.useState('');
  const [grantRole, setGrantRole] = React.useState<'owner' | 'manager' | 'viewer'>('viewer');
  const [invitation, invitationEvent] = React.useReducer(invitationDraftTransition, emptyInvitation);
  const { open: inviteOpen, email: inviteEmail, vehicleIds: inviteVehicleIds, authMethods: inviteAuthMethods, activationLink } = invitation;
  const setInviteEmail = (email: string) => invitationEvent({ type: 'edit', patch: { email } });
  const setInviteVehicleIds = (vehicleIds: string[]) => invitationEvent({ type: 'edit', patch: { vehicleIds } });
  const setInviteAuthMethods = (authMethods: InvitationAuthMethods) => invitationEvent({ type: 'edit', patch: { authMethods } });
  function openInvitation() { action.cancel(); invitationEvent({ type: 'open' }); }
  function closeInvitation() { action.cancel(); invitationEvent({ type: 'close' }); }
  const [confirmation, setConfirmation] = React.useState<Confirmation | null>(null);

  const users = useQuery({
    queryKey: ['admin-users', search],
    queryFn: () => api.listUsers(search.trim()),
    enabled: authReady && isPrivileged && !!accessToken,
    refetchOnMount: 'always',
  });
  const accountInvitations = useQuery({
    queryKey: ['admin-account-invitations'],
    queryFn: () => api.listAccountInvitations(),
    enabled: authReady && isPrivileged && !!accessToken,
    refetchOnMount: 'always',
  });
  const vehicleOptions = useQuery({
    queryKey: ['admin-vehicle-options'],
    queryFn: () => api.listAdminVehicleOptions(),
    enabled: authReady && isPrivileged && !!accessToken,
    refetchOnMount: 'always',
  });
  const authConfig = useQuery({
    queryKey: ['auth-config'],
    queryFn: () => api.getAuthConfig(),
    enabled: authReady && isPrivileged,
    refetchOnMount: 'always',
  });
  const availableMethods = availableInviteMethods(authConfig.data);
  const selectedInviteMethods = inviteAuthMethods && availableMethods.includes(inviteAuthMethods)
    ? inviteAuthMethods
    : availableMethods[0] ?? 'password';

  React.useEffect(() => {
    let available = users.data?.[0]?.id ?? null;
    for (const account of users.data ?? []) {
      if (account.id !== selectedUserId) continue;
      available = account.id;
      break;
    }
    if (available !== selectedUserId) setSelectedUserId(available);
  }, [selectedUserId, users.data]);

  const selectedUser = users.data?.find((user) => user.id === selectedUserId) ?? null;
  const detail = useQuery({
    queryKey: ['admin-user-detail', selectedUserId],
    queryFn: () => api.getUserDetail(selectedUserId!),
    enabled: authReady && isPrivileged && !!selectedUserId && !!accessToken,
    refetchOnMount: 'always',
  });

  React.useEffect(() => {
    if (detail.data) accountEvent({ type: 'load', user: detail.data.user });
  }, [detail.data?.user.id, detail.data?.user.email, detail.data?.user.role]);

  const createInvitation = useMutation({
    mutationFn: ({ email, vehicleIds, authMethods }: { generation: number; email: string; vehicleIds: string[]; authMethods: InvitationAuthMethods }) => api.createAccountInvitation({ email, vehicle_ids: vehicleIds, auth_methods: authMethods }),
    onSuccess: (result, variables) => {
      invitationEvent({ type: 'created', generation: variables.generation, link: `${window.location.origin}/activate#${result.activation_token}` });
      void queryClient.invalidateQueries({ queryKey: ['admin-account-invitations'] });
      void queryClient.invalidateQueries({ queryKey: ['admin-users'] });
    },
  });
  const updateUser = useMutation({
    mutationFn: ({ id, body }: { id: string; body: { email?: string; role?: UserRole; is_disabled?: boolean } }) => api.updateUser(id, body),
    onSuccess: (_result, variables) => {
      void queryClient.invalidateQueries({ queryKey: ['admin-users'] });
      void queryClient.invalidateQueries({ queryKey: ['admin-user-detail', variables.id] });
    },
  });
  const deleteUser = useMutation({
    mutationFn: (id: string) => api.deleteUser(id),
    onSuccess: (_result, userId) => {
      setSelectedUserId((current) => current === userId ? null : current);
      void queryClient.invalidateQueries({ queryKey: ['admin-users'] });
    },
  });
  const revokeAccountInvitation = useMutation({
    mutationFn: (id: string) => api.revokeAccountInvitation(id),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ['admin-account-invitations'] });
      void queryClient.invalidateQueries({ queryKey: ['admin-users'] });
    },
  });
  const grantMembership = useMutation({
    mutationFn: ({ userId, vehicleId, role }: { userId: string; vehicleId: string; role: 'owner' | 'manager' | 'viewer' }) =>
      api.grantUserVehicleMembership(userId, vehicleId, role),
    onSuccess: (_result, variables) => {
      if (variables.userId === selectedUserId) setGrantVehicleId('');
      void queryClient.invalidateQueries({ queryKey: ['admin-user-detail', variables.userId] });
      void queryClient.invalidateQueries({ queryKey: ['admin-users'] });
    },
  });
  const updateMembership = useMutation({
    mutationFn: ({ userId, vehicleId, role }: { userId: string; vehicleId: string; role: 'owner' | 'manager' | 'viewer' }) =>
      api.updateUserVehicleMembership(userId, vehicleId, role),
    onSuccess: (_result, variables) => void queryClient.invalidateQueries({ queryKey: ['admin-user-detail', variables.userId] }),
  });
  const removeMembership = useMutation({
    mutationFn: ({ userId, vehicleId }: { userId: string; vehicleId: string }) => api.removeUserVehicleMembership(userId, vehicleId),
    onSuccess: (_result, variables) => void queryClient.invalidateQueries({ queryKey: ['admin-user-detail', variables.userId] }),
  });
  const revokeVehicleInvitation = useMutation({
    mutationFn: ({ userId, invitationId }: { userId: string; invitationId: string }) => api.revokeUserInvite(userId, invitationId),
    onSuccess: (_result, variables) => void queryClient.invalidateQueries({ queryKey: ['admin-user-detail', variables.userId] }),
  });

  const invitationGroups = (accountInvitations.data ?? []).reduce((groups, invitation) => {
    groups[invitationStatus(invitation) === 'pending' ? 'pending' : 'history'].push(invitation);
    return groups;
  }, { pending: [] as NonNullable<typeof accountInvitations.data>, history: [] as NonNullable<typeof accountInvitations.data> });
  const pendingInvitations = invitationGroups.pending;
  const invitationHistory = invitationGroups.history;

  async function performAction(work: () => Promise<unknown>, success: [string, string], failure: string, commit?: () => void) {
    const active = action.begin();
    try {
      await work();
      if (!active()) return;
      commit?.();
      emitToast(...success, 'success');
    } catch (error) {
      if (active()) emitToast(failure, errorMessage(error), 'error');
    }
  }

  function handleInviteSubmit() {
    return performAction(() => createInvitation.mutateAsync({ generation: invitation.generation, email: inviteEmail.trim(), vehicleIds: inviteVehicleIds, authMethods: selectedInviteMethods }), ['Invitation created', 'Copy the activation link before closing this dialog.'], 'Invitation failed');
  }

  function handleAccountSave() {
    const user = detail.data?.user;
    if (!user) return;
    const email = accountDraft.email.trim();
    const body = {
      ...(email && email !== user.email ? { email } : {}),
      ...(accountDraft.role !== user.role ? { role: accountDraft.role } : {}),
    };
    if (!Object.keys(body).length) { accountEvent({ type: 'saved', userId: user.id }); return; }
    return performAction(() => updateUser.mutateAsync({ id: user.id, body }), ['Account updated', 'The account details have been saved.'], 'Account update failed', () => accountEvent({ type: 'saved', userId: user.id }));
  }

  function handleConfirmation() {
    if (!confirmation) return;
    const pending = confirmation;
    const command = pending.kind === 'delete-user'
      ? { work: () => deleteUser.mutateAsync(pending.userId), success: ['Account deleted', `${pending.email} has been removed.`] as [string, string] }
      : pending.kind === 'revoke-account-invitation'
        ? { work: () => revokeAccountInvitation.mutateAsync(pending.invitationId), success: ['Invitation revoked', `The invitation for ${pending.email} is no longer active.`] as [string, string] }
        : { work: () => revokeVehicleInvitation.mutateAsync({ userId: pending.userId, invitationId: pending.invitationId }), success: ['Invitation revoked', `The ${pending.vehicleName} invitation is no longer active.`] as [string, string] };
    return performAction(command.work, command.success, 'Action failed', () => setConfirmation((current) => current === pending ? null : current));
  }

  const confirmationPending = deleteUser.isPending || revokeAccountInvitation.isPending || revokeVehicleInvitation.isPending;

  return (
    <AppLayout activeKey="users">
      <PageLayout
        title="Users"
        subtitle="Manage accounts, vehicle access, and invitations."
        actions={isPrivileged ? (
          <Button type="button" size="md" iconLeft={<MailPlus className="h-4 w-4" />} onClick={openInvitation}>
            Invite user
          </Button>
        ) : undefined}
      >
        {!isPrivileged ? (
          <Card><CardContent><p className="text-sm text-fg-tertiary">Admin access is required.</p></CardContent></Card>
        ) : (
          <div className="grid gap-5 lg:grid-cols-[13rem_minmax(0,1fr)]">
            <nav className="flex gap-2 overflow-x-auto lg:flex-col lg:overflow-visible" aria-label="User management sections">
              <button type="button" onClick={() => setSection('users')} className={`flex h-9 shrink-0 items-center gap-2 rounded-lg px-3 text-left text-sm font-medium transition-colors ${section === 'users' ? 'bg-accent text-fg-on-accent shadow-glow-button' : 'text-fg-secondary hover:bg-bg-elevated hover:text-fg'}`}>
                <UsersRound className="h-4 w-4" /> Users
              </button>
              <button type="button" aria-label="Invitations" onClick={() => setSection('invitations')} className={`flex h-9 shrink-0 items-center justify-between gap-2 rounded-lg px-3 text-left text-sm font-medium transition-colors ${section === 'invitations' ? 'bg-accent text-fg-on-accent shadow-glow-button' : 'text-fg-secondary hover:bg-bg-elevated hover:text-fg'}`}>
                <span className="flex items-center gap-2"><MailPlus className="h-4 w-4" /> Invitations</span>
                {pendingInvitations.length > 0 ? <Badge size="sm" variant="warning">{pendingInvitations.length}</Badge> : null}
              </button>
            </nav>

            {section === 'users' ? (
              <Card padding="none" className="overflow-hidden">
                <div className="grid min-h-[34rem] lg:grid-cols-[20rem_minmax(0,1fr)]">
                  <section className="border-b border-border p-4 lg:border-b-0 lg:border-r" aria-label="Accounts">
                    <label className="sr-only" htmlFor="user-search">Search accounts</label>
                    <input id="user-search" value={search} onChange={(event) => setSearch(event.target.value)} placeholder="Search accounts" className={`${CONTROL_CLASS} w-full`} />
                    <div className="mt-3 divide-y divide-border overflow-hidden rounded-lg border border-border">
                      {users.isLoading ? <p className="p-4 text-sm text-fg-tertiary">Loading accounts…</p> : null}
                      {!users.isLoading && (users.data?.length ?? 0) === 0 ? <p className="p-4 text-sm text-fg-tertiary">No accounts found.</p> : null}
                      {users.data?.map((user) => (
                        <button key={user.id} type="button" onClick={() => setSelectedUserId(user.id)} className={`flex w-full items-center justify-between gap-3 px-3 py-3 text-left transition-colors ${selectedUserId === user.id ? 'bg-accent-muted/35' : 'hover:bg-bg-elevated/70'}`}>
                          <span className="min-w-0"><span className="block truncate text-sm font-medium text-fg">{user.email}</span><span className="mt-1 flex flex-wrap items-center gap-1.5"><Badge size="sm">{user.role}</Badge><Badge size="sm" variant={user.is_disabled ? 'warning' : 'success'}>{user.is_disabled ? 'disabled' : 'active'}</Badge></span></span>
                          <span className="shrink-0 text-xs text-fg-tertiary">{user.vehicle_count} vehicle{user.vehicle_count === 1 ? '' : 's'}</span>
                        </button>
                      ))}
                    </div>
                  </section>

                  <section className="p-4" aria-label="Selected user">
                    {!selectedUser ? <EmptyState icon={<UserRound />} title="Select an account" description="Choose an account to manage its details and vehicle access." className="min-h-72" /> : detail.isLoading || !detail.data ? <p className="p-4 text-sm text-fg-tertiary">Loading account details…</p> : (
                      <div className="grid gap-5">
                        <div className="flex flex-wrap items-center justify-between gap-3">
                          <div className="min-w-0"><h2 className="truncate text-lg font-semibold text-fg">{detail.data.user.email}</h2><div className="mt-1 flex flex-wrap gap-1.5"><Badge>{detail.data.user.role}</Badge><Badge variant={detail.data.user.is_disabled ? 'warning' : 'success'}>{detail.data.user.is_disabled ? 'disabled' : 'active'}</Badge></div></div>
                          <div className="flex items-center gap-2">
                            <IconAction label={detail.data.user.is_disabled ? 'Enable account' : 'Disable account'} onClick={() => { const user = detail.data!.user; void performAction(() => updateUser.mutateAsync({ id: user.id, body: { is_disabled: !user.is_disabled } }), [user.is_disabled ? 'Account enabled' : 'Account disabled', user.email], 'Account update failed'); }}>
                              {detail.data.user.is_disabled ? <Check className="h-4 w-4" /> : <Ban className="h-4 w-4" />}
                            </IconAction>
                            {isSuperUser ? <IconAction label="Delete account" variant="danger" onClick={() => setConfirmation({ kind: 'delete-user', userId: detail.data!.user.id, email: detail.data!.user.email })}><Trash2 className="h-4 w-4" /></IconAction> : null}
                          </div>
                        </div>

                        <div className="flex flex-wrap gap-2" role="tablist" aria-label="Account details">
                          {([['account', 'Account'], ['vehicles', 'Vehicles'], ['invites', 'Invites']] as const).map(([tab, label]) => <Button key={tab} type="button" size="md" variant={detailTab === tab ? 'primary' : 'secondary'} role="tab" aria-selected={detailTab === tab} onClick={() => accountEvent({ type: 'tab', tab })}>{label}</Button>)}
                        </div>

                        {detailTab === 'account' ? (
                          isEditingAccount ? (
                            <div className="grid gap-3 rounded-xl border border-border bg-bg-elevated/35 p-4 sm:grid-cols-[minmax(0,1fr)_11rem_auto_auto]">
                              <label className="sr-only" htmlFor="account-email">Account email</label><input id="account-email" value={accountDraft.email} onChange={(event) => accountEvent({ type: 'edit', patch: { email: event.target.value } })} className={CONTROL_CLASS} />
                              <label className="sr-only" htmlFor="account-role">Account role</label><SelectPicker id="account-role" value={accountDraft.role} disabled={!isSuperUser} onChange={(value) => accountEvent({ type: 'edit', patch: { role: value as UserRole } })} aria-label="Account role" options={[{ value: 'user', label: 'user' }, { value: 'admin', label: 'admin' }, { value: 'super_user', label: 'super_user' }]} />
                              <Button type="button" size="md" loading={updateUser.isPending} onClick={() => void handleAccountSave()}>Save</Button>
                              <Button type="button" size="md" variant="secondary" onClick={() => accountEvent({ type: 'cancel' })}>Cancel</Button>
                            </div>
                          ) : (
                            <div className="flex items-center justify-between gap-3 rounded-xl border border-border bg-bg-elevated/35 p-4"><dl className="grid gap-2 text-sm"><div className="grid gap-1 sm:grid-cols-[6rem_1fr]"><dt className="text-fg-tertiary">Email</dt><dd className="text-fg">{detail.data.user.email}</dd></div><div className="grid gap-1 sm:grid-cols-[6rem_1fr]"><dt className="text-fg-tertiary">Role</dt><dd className="text-fg">{detail.data.user.role}</dd></div></dl><IconAction label="Edit account" onClick={() => accountEvent({ type: 'begin' })}><Pencil className="h-4 w-4" /></IconAction></div>
                          )
                        ) : null}

                        {detailTab === 'vehicles' ? (
                          <div className="grid gap-4">
                            <div className="grid gap-2 rounded-xl border border-border bg-bg-elevated/35 p-4 sm:grid-cols-[minmax(0,1fr)_10rem_auto]">
                              <label className="sr-only" htmlFor="grant-vehicle">Vehicle</label><SelectPicker id="grant-vehicle" value={grantVehicleId} onChange={setGrantVehicleId} aria-label="Vehicle" className="min-w-0 flex-1" options={[{ value: '', label: 'Select a vehicle' }, ...(vehicleOptions.data?.map((vehicle) => ({ value: vehicle.id, label: `${vehicle.display_name} · ${vehicle.model}` })) ?? [])]} />
                              <label className="sr-only" htmlFor="grant-role">Membership role</label><SelectPicker id="grant-role" value={grantRole} onChange={(value) => setGrantRole(value as 'owner' | 'manager' | 'viewer')} aria-label="Membership role" options={[{ value: 'owner', label: 'owner' }, { value: 'manager', label: 'manager' }, { value: 'viewer', label: 'viewer' }]} />
                              <Button type="button" size="md" loading={grantMembership.isPending} disabled={!grantVehicleId} onClick={() => { const userId = detail.data?.user.id; if (userId) void performAction(() => grantMembership.mutateAsync({ userId, vehicleId: grantVehicleId, role: grantRole }), ['Vehicle access granted', 'The membership has been saved.'], 'Vehicle access failed'); }}>Grant access</Button>
                            </div>
                            <div className="divide-y divide-border overflow-hidden rounded-xl border border-border">
                              {detail.data.memberships.length === 0 ? <p className="p-4 text-sm text-fg-tertiary">No vehicle access has been assigned.</p> : detail.data.memberships.map((membership) => <div key={membership.vehicle_id} className="flex flex-wrap items-center justify-between gap-3 p-4"><div><p className="text-sm font-medium text-fg">{membership.display_name || membership.model}</p><div className="mt-1 flex items-center gap-2"><Badge size="sm">{membership.role}</Badge>{membership.is_default ? <Badge size="sm" variant="success">default</Badge> : null}</div></div><div className="flex items-center gap-2"><label className="sr-only" htmlFor={`membership-role-${membership.vehicle_id}`}>Role for {membership.display_name || membership.model}</label><SelectPicker id={`membership-role-${membership.vehicle_id}`} value={membership.role} onChange={(value) => { const userId = detail.data!.user.id; void performAction(() => updateMembership.mutateAsync({ userId, vehicleId: membership.vehicle_id, role: value as 'owner' | 'manager' | 'viewer' }), ['Vehicle role updated', 'The membership role has been saved.'], 'Vehicle role failed'); }} aria-label={`Role for ${membership.display_name || membership.model}`} className="w-28" size="sm" options={[{ value: 'owner', label: 'owner' }, { value: 'manager', label: 'manager' }, { value: 'viewer', label: 'viewer' }]} /><IconAction label={`Remove ${membership.display_name || membership.model} access`} variant="danger" onClick={() => { const userId = detail.data!.user.id; void performAction(() => removeMembership.mutateAsync({ userId, vehicleId: membership.vehicle_id }), ['Vehicle access removed', 'The membership has been removed.'], 'Vehicle removal failed'); }}><Trash2 className="h-4 w-4" /></IconAction></div></div>)}
                            </div>
                          </div>
                        ) : null}

                        {detailTab === 'invites' ? <div className="divide-y divide-border overflow-hidden rounded-xl border border-border">{detail.data.invites.length === 0 ? <p className="p-4 text-sm text-fg-tertiary">No vehicle invitations for this account.</p> : detail.data.invites.map((invitation) => { const status = invitationStatus(invitation); return <div key={invitation.id} className="flex flex-wrap items-center justify-between gap-3 p-4"><div><p className="text-sm font-medium text-fg">{invitation.vehicle_name}</p><p className="mt-1 text-xs text-fg-tertiary">{status === 'pending' ? `Expires ${formatAppDateTime(invitation.expires_at)}` : status}</p></div><div className="flex items-center gap-2"><Badge size="sm" variant={status === 'pending' ? 'warning' : status === 'accepted' ? 'success' : 'danger'}>{status}</Badge>{status === 'pending' ? <IconAction label={`Revoke ${invitation.vehicle_name} invitation`} variant="danger" onClick={() => setConfirmation({ kind: 'revoke-vehicle-invitation', userId: detail.data!.user.id, invitationId: invitation.id, vehicleName: invitation.vehicle_name })}><Trash2 className="h-4 w-4" /></IconAction> : null}</div></div>; })}</div> : null}
                      </div>
                    )}
                  </section>
                </div>
              </Card>
            ) : (
              <Card padding="none" className="overflow-hidden">
                <section className="p-4 sm:p-5" aria-label="Account invitations">
                  <div className="flex flex-wrap items-center justify-between gap-3"><div><h2 className="text-base font-semibold text-fg">Account invitations</h2><p className="mt-1 text-sm text-fg-tertiary">Pending invitations are actionable; completed invitations remain available as history.</p></div><Button type="button" size="md" iconLeft={<MailPlus className="h-4 w-4" />} onClick={openInvitation}>Invite user</Button></div>
                  <div className="mt-5 divide-y divide-border overflow-hidden rounded-xl border border-border">{pendingInvitations.length === 0 ? <p className="p-4 text-sm text-fg-tertiary">No pending account invitations.</p> : pendingInvitations.map((invitation) => <div key={invitation.id} className="flex flex-wrap items-center justify-between gap-3 p-4"><div><p className="text-sm font-medium text-fg">{invitation.invitee_email}</p><p className="mt-1 text-xs text-fg-tertiary">Expires {formatAppDateTime(invitation.expires_at)}</p><p className="mt-1 text-xs text-fg-tertiary">Sign-in: {AUTH_METHOD_LABELS[invitation.auth_methods]}</p><p className="mt-1 text-xs text-fg-tertiary">Vehicle access: {invitation.vehicle_names.length > 0 ? invitation.vehicle_names.join(', ') : 'None'}</p></div><div className="flex items-center gap-2"><Badge size="sm" variant="warning">pending</Badge><IconAction label={`Revoke invitation for ${invitation.invitee_email}`} variant="danger" onClick={() => setConfirmation({ kind: 'revoke-account-invitation', invitationId: invitation.id, email: invitation.invitee_email })}><Trash2 className="h-4 w-4" /></IconAction></div></div>)}</div>
                  {invitationHistory.length > 0 ? <details className="mt-4 rounded-xl border border-border bg-bg-elevated/35"><summary className="cursor-pointer px-4 py-3 text-sm font-medium text-fg">Invitation history ({invitationHistory.length})</summary><div className="divide-y divide-border border-t border-border">{invitationHistory.map((invitation) => { const status = invitationStatus(invitation); return <div key={invitation.id} className="flex flex-wrap items-center justify-between gap-3 p-4 text-sm"><div><span className="text-fg">{invitation.invitee_email}</span><p className="mt-1 text-xs text-fg-tertiary">Sign-in: {AUTH_METHOD_LABELS[invitation.auth_methods]}</p><p className="mt-1 text-xs text-fg-tertiary">Vehicle access: {invitation.vehicle_names.length > 0 ? invitation.vehicle_names.join(', ') : 'None'}</p></div><Badge size="sm" variant={status === 'accepted' ? 'success' : 'danger'}>{status}</Badge></div>; })}</div></details> : null}
                </section>
              </Card>
            )}
          </div>
        )}
      </PageLayout>
      {inviteOpen ? <InviteUserDialog email={inviteEmail} setEmail={setInviteEmail} vehicleIds={inviteVehicleIds} setVehicleIds={setInviteVehicleIds} vehicleOptions={vehicleOptions.data ?? []} authMethods={selectedInviteMethods} setAuthMethods={setInviteAuthMethods} availableMethods={availableMethods} authOptionsLoading={authConfig.isPending} authOptionsError={authConfig.isError} activationLink={activationLink} pending={createInvitation.isPending} onClose={closeInvitation} onSubmit={() => void handleInviteSubmit()} onCopy={() => { if (activationLink) { void navigator.clipboard?.writeText(activationLink); emitToast('Activation link copied', 'The invitation link is ready to share.', 'success'); } }} /> : null}
      {confirmation ? <ConfirmationDialog confirmation={confirmation} pending={confirmationPending} onCancel={() => setConfirmation(null)} onConfirm={() => void handleConfirmation()} /> : null}
    </AppLayout>
  );
}
