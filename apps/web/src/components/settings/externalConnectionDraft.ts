import type { SetStateAction } from 'react';
import type { UpdateExternalConnectionBody } from '@panda/types';

export interface ExternalConnectionDraft {
  draft: UpdateExternalConnectionBody;
  apiKey: string;
  bearerToken: string;
  isReplacingApiKey: boolean;
  isClearingApiKey: boolean;
  message: string | null;
  previewDataUrl: string | null;
}
export function initialExternalConnectionDraft(draft: UpdateExternalConnectionBody): ExternalConnectionDraft {
  return { draft, apiKey: '', bearerToken: '', isReplacingApiKey: false, isClearingApiKey: false, message: null, previewDataUrl: null };
}
type ConnectionDraftEvent =
  | { type: 'load'; draft: UpdateExternalConnectionBody }
  | { type: 'draft'; value: SetStateAction<UpdateExternalConnectionBody> }
  | { type: 'secret'; key: 'apiKey' | 'bearerToken'; value: string }
  | { type: 'replace' | 'saved' }
  | { type: 'clearing'; value: boolean }
  | { type: 'feedback'; message: string }
  | { type: 'tested'; message: string; preview: string | null };
export function externalConnectionDraftTransition(state: ExternalConnectionDraft, event: ConnectionDraftEvent): ExternalConnectionDraft {
  switch (event.type) {
    case 'load': return { ...initialExternalConnectionDraft(event.draft), message: state.message };
    case 'draft': return { ...state, draft: typeof event.value === 'function' ? event.value(state.draft) : event.value };
    case 'secret': return { ...state, [event.key]: event.value };
    case 'replace': return { ...state, draft: { ...state.draft, clear_api_key: false }, isReplacingApiKey: true };
    case 'clearing': return { ...state, isClearingApiKey: event.value };
    case 'saved': return { ...state, apiKey: '', isReplacingApiKey: false, isClearingApiKey: false, message: 'Saved.' };
    case 'feedback': return { ...state, message: event.message };
    case 'tested': return { ...state, message: event.message, previewDataUrl: event.preview };
  }
}
