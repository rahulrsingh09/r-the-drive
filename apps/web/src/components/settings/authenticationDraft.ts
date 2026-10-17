import type { AuthenticationSettingsUpdate } from '@panda/types';

type Field = keyof AuthenticationSettingsUpdate;
type Setting = { value: unknown; source?: unknown };
export interface AuthenticationFormState {
  form: AuthenticationSettingsUpdate;
  secret: string;
  removeSecret: boolean;
  showAdvancedAccess: boolean;
  message: string;
  messageRole: 'status' | 'alert';
}
type FormEvent =
  | { type: 'load'; form: AuthenticationSettingsUpdate; advanced: boolean }
  | { type: 'field'; key: Field; value: unknown }
  | { type: 'secret'; value: string }
  | { type: 'remove-secret'; value: boolean }
  | { type: 'advanced' | 'saved' }
  | { type: 'feedback'; message: string; role: 'status' | 'alert' };
export function authenticationFormTransition(state: AuthenticationFormState, event: FormEvent): AuthenticationFormState {
  switch (event.type) {
    case 'load': return { ...state, form: event.form, showAdvancedAccess: event.advanced };
    case 'field': return { ...state, message: '', messageRole: 'status', form: { ...state.form, [event.key]: event.value } };
    case 'secret': return { ...state, message: '', messageRole: 'status', secret: event.value, removeSecret: event.value.trim() ? false : state.removeSecret };
    case 'remove-secret': return { ...state, message: '', messageRole: 'status', secret: '', removeSecret: event.value };
    case 'advanced': return { ...state, showAdvancedAccess: !state.showAdvancedAccess };
    case 'saved': return { ...state, secret: '', removeSecret: false, messageRole: 'status', message: 'Authentication settings saved.' };
    case 'feedback': return { ...state, message: event.message, messageRole: event.role };
  }
}

export function authenticationDraft(data: unknown, fields: readonly Field[], defaults: AuthenticationSettingsUpdate) {
  const record = data as Record<Field, unknown>;
  const settings = new Map<Field, Setting>();
  for (const field of fields) {
    const candidate = record[field];
    if (candidate && typeof candidate === 'object' && 'value' in candidate) settings.set(field, candidate as Setting);
  }
  const form = Object.assign({}, defaults, Object.fromEntries([...settings].map(([key, setting]) => [key, setting.value]))) as AuthenticationSettingsUpdate;
  return {
    form,
    advanced: Boolean(form.allowed_email_domains?.length || form.required_claim_name || form.required_claim_value),
    source: (key: Field) => {
      const candidate = record[key];
      return candidate && typeof candidate === 'object' && 'source' in candidate ? String(candidate.source) : undefined;
    },
    patch(current: AuthenticationSettingsUpdate, secret: string, removeSecret: boolean) {
      const changed = fields.filter((key) => !(record[key] && typeof record[key] === 'object' && 'source' in record[key] && (record[key] as { source: unknown }).source === 'environment') && JSON.stringify(current[key]) !== JSON.stringify(settings.get(key)?.value));
      const body = Object.fromEntries(changed.map((key) => [key, current[key]])) as AuthenticationSettingsUpdate;
      if (removeSecret || secret.trim()) body.client_secret = removeSecret ? null : secret;
      return body;
    },
  };
}
