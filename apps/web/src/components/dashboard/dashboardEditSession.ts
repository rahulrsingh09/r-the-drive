import type { SetStateAction } from 'react';
import type { DashboardConfig } from '@panda/dashboards';

export interface DashboardEditInput {
  key: string;
  saved: DashboardConfig | undefined;
  editing: boolean;
}

type Draft =
  | { kind: 'empty'; config: null }
  | { kind: 'pristine' | 'dirty'; config: DashboardConfig };

export interface DashboardEditSession {
  input: DashboardEditInput;
  draft: Draft;
  error: string | null;
}

type DashboardEditEvent =
  | { type: 'observe'; input: DashboardEditInput }
  | { type: 'enter' }
  | { type: 'exit' }
  | { type: 'edit'; update: SetStateAction<DashboardConfig | null> }
  | { type: 'error'; update: SetStateAction<string | null> };

function classifyDraft(config: DashboardConfig | null, saved: DashboardConfig | undefined): Draft {
  if (!config) return { kind: 'empty', config: null };
  // Keep the existing saved-config equality contract, including nested options.
  const differs = saved && JSON.stringify(config) !== JSON.stringify(saved);
  return { kind: differs ? 'dirty' : 'pristine', config };
}

export function createDashboardEditSession(input: DashboardEditInput): DashboardEditSession {
  return {
    input,
    draft: classifyDraft(input.editing ? input.saved ?? null : null, input.saved),
    error: null,
  };
}

export function dashboardEditSessionReducer(
  session: DashboardEditSession,
  event: DashboardEditEvent,
): DashboardEditSession {
  switch (event.type) {
    case 'observe': {
      const input = event.input;
      if (input.key !== session.input.key || input.editing !== session.input.editing) {
        return createDashboardEditSession(input);
      }
      if (input.saved === session.input.saved) return session;

      // Decide whether to follow the refresh using the previous draft state.
      // Comparing an untouched draft to the new saved value would invent edits.
      const followSaved = input.editing && session.draft.kind === 'pristine';
      return {
        input,
        draft: classifyDraft(followSaved ? input.saved ?? null : session.draft.config, input.saved),
        error: session.error,
      };
    }
    case 'enter':
      return {
        ...session,
        draft: classifyDraft(session.input.saved ?? null, session.input.saved),
        error: null,
      };
    case 'exit':
      return { ...session, draft: { kind: 'empty', config: null }, error: null };
    case 'edit': {
      const config = typeof event.update === 'function'
        ? event.update(session.draft.config)
        : event.update;
      return { ...session, draft: classifyDraft(config, session.input.saved), error: null };
    }
    case 'error':
      return {
        ...session,
        error: typeof event.update === 'function' ? event.update(session.error) : event.update,
      };
  }
}
