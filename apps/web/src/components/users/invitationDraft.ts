import type { InvitationAuthMethods } from '@panda/types';

export interface InvitationDraft {
  generation: number;
  open: boolean;
  email: string;
  vehicleIds: string[];
  authMethods: InvitationAuthMethods | null;
  activationLink: string | null;
}
export const emptyInvitation: InvitationDraft = {
  generation: 0, open: false, email: '', vehicleIds: [], authMethods: null, activationLink: null,
};
type InvitationEvent =
  | { type: 'open' | 'close' }
  | { type: 'edit'; patch: Partial<Pick<InvitationDraft, 'email' | 'vehicleIds' | 'authMethods'>> }
  | { type: 'created'; generation: number; link: string };
export function invitationDraftTransition(state: InvitationDraft, event: InvitationEvent): InvitationDraft {
  switch (event.type) {
    case 'open': return { ...emptyInvitation, generation: state.generation + 1, open: true };
    case 'close': return { ...state, open: false, generation: state.generation + 1 };
    case 'edit': return { ...state, ...event.patch };
    case 'created': return state.open && state.generation === event.generation ? { ...state, activationLink: event.link } : state;
  }
}

type DialogEvent = 'continue' | 'back' | 'copied';
export function invitationDialogTransition(state: { step: 'email' | 'vehicle'; copied: boolean }, event: DialogEvent) {
  if (event === 'copied') return { ...state, copied: true };
  return { ...state, step: event === 'continue' ? 'vehicle' as const : 'email' as const };
}
