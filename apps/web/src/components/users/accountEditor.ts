import type { UserRole } from '@panda/types';

type Account = { id: string; email: string; role: UserRole };
interface AccountEditor {
  source: Account | null;
  draft: { email: string; role: UserRole };
  editing: boolean;
  tab: 'account' | 'vehicles' | 'invites';
}
export const emptyAccountEditor: AccountEditor = { source: null, draft: { email: '', role: 'user' }, editing: false, tab: 'account' };
type AccountEvent =
  | { type: 'load'; user: Account }
  | { type: 'edit'; patch: Partial<AccountEditor['draft']> }
  | { type: 'begin' | 'cancel' }
  | { type: 'tab'; tab: AccountEditor['tab'] }
  | { type: 'saved'; userId: string };
export function accountEditorTransition(state: AccountEditor, event: AccountEvent): AccountEditor {
  switch (event.type) {
    case 'load': {
      const { id, email, role } = event.user;
      if (state.source?.id === id && state.source.email === email && state.source.role === role) return state;
      return { source: { id, email, role }, draft: { email, role }, editing: false, tab: 'account' };
    }
    case 'edit': return { ...state, draft: { ...state.draft, ...event.patch } };
    case 'tab': return { ...state, tab: event.tab };
    case 'begin': return { ...state, editing: true };
    case 'cancel': return { ...state, editing: false, draft: state.source ? { email: state.source.email, role: state.source.role } : state.draft };
    case 'saved': return state.source?.id === event.userId ? { ...state, editing: false } : state;
  }
}
